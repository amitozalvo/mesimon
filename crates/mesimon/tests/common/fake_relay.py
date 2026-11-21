#!/usr/bin/env python3
"""Minimal local loopback stand-in for `mesimon_team`'s public client wire
contract (used in `agy_e2e.rs` and local browser verification). Zero relay
internals: routes opaque JSON frames between one local host daemon and one or
more local browser clients on 127.0.0.1.
"""
import base64
import hashlib
import json
import mimetypes
import os
import secrets
import socket
import ssl
import struct
import subprocess
import sys
import threading
import traceback

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MIME = {
    ".html": "text/html; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".woff2": "font/woff2",
    ".webmanifest": "application/manifest+json",
    ".png": "image/png",
    ".wasm": "application/wasm",
}


def device_id_of(public):
    h = hashlib.sha256()
    h.update(b"mesimon-team device v1\x00")
    h.update(bytes.fromhex(public["sign"]))
    return h.digest()[:16].hex()


def ws_send(sock, lock, payload_str):
    raw = payload_str.encode("utf-8")
    n = len(raw)
    if n < 126:
        hdr = bytes([0x81, n])
    elif n < 65536:
        hdr = struct.pack("!BBH", 0x81, 126, n)
    else:
        hdr = struct.pack("!BBQ", 0x81, 127, n)
    with lock:
        sock.sendall(hdr + raw)


def recv_exact(sock, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError
        buf.extend(chunk)
    return bytes(buf)


def ws_recv(sock, lock):
    while True:
        b0, b1 = recv_exact(sock, 2)
        opcode = b0 & 0x0F
        masked = (b1 & 0x80) != 0
        ln = b1 & 0x7F
        if ln == 126:
            (ln,) = struct.unpack("!H", recv_exact(sock, 2))
        elif ln == 127:
            (ln,) = struct.unpack("!Q", recv_exact(sock, 8))
        mask = recv_exact(sock, 4) if masked else b""
        data = bytearray(recv_exact(sock, ln))
        if masked:
            for i in range(ln):
                data[i] ^= mask[i & 3]
        if opcode == 0x8:  # Close
            return None
        if opcode == 0x9:  # Ping -> Pong
            with lock:
                sock.sendall(bytes([0x8A, len(data)]) + bytes(data))
            continue
        if opcode == 0xA:  # Pong
            continue
        if opcode == 0x1:  # Text
            return data.decode("utf-8")


def main():
    state_dir = sys.argv[1]
    web_root = sys.argv[2] if len(sys.argv) > 2 else ""
    cert_pem = os.path.join(state_dir, "relay-cert.pem")
    key_pem = os.path.join(state_dir, "relay-key.pem")
    subprocess.run(
        ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", key_pem],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    subprocess.run(
        [
            "openssl", "req", "-new", "-x509", "-key", key_pem, "-out", cert_pem,
            "-days", "1", "-subj", "/CN=127.0.0.1",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    der = ssl.PEM_cert_to_DER_cert(open(cert_pem).read())
    pin = hashlib.sha256(der).hexdigest()

    tls_srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    tls_srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    tls_srv.bind(("127.0.0.1", 0))
    tls_srv.listen(16)
    tls_port = tls_srv.getsockname()[1]

    http_srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    http_srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    http_srv.bind(("127.0.0.1", 0))
    http_srv.listen(16)
    http_port = http_srv.getsockname()[1]
    origin = f"http://127.0.0.1:{http_port}"

    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(cert_pem, key_pem)

    mu = threading.Lock()
    state = {
        "host": None,          # (sock, lock)
        "host_device": "00" * 16,
        "board": None,
        "invites": set(),
        "devices": set(),
        "browsers": {},        # cred_hex -> {"device": dev_hex, "name": str, "public": dict}
        "peers": {},           # peer_id -> (sock, lock)
        "next_peer": 1,
    }

    def handle_tls(raw):
        try:
            with ctx.wrap_socket(raw, server_side=True) as conn:
                line = b""
                while not line.endswith(b"\n"):
                    chunk = conn.recv(4096)
                    if not chunk:
                        return
                    line += chunk
                frame = json.loads(line.decode("utf-8"))
                req = frame.get("request", {})
                kind = req if isinstance(req, str) else req.get("kind")
                if kind == "control_info":
                    resp = {"kind": "control_info", "version": 1, "origin": origin}
                elif kind == "control_mail":
                    resp = {"kind": "control_mail", "version": 1}
                elif kind == "control_shelf":
                    resp = {"kind": "control_shelf", "version": 1}
                elif kind == "boards":
                    resp = {"kind": "boards", "boards": []}
                else:
                    resp = {"kind": "error", "code": "invalid_request"}
                conn.sendall((json.dumps(resp) + "\n").encode("utf-8"))
                try:
                    conn.unwrap()
                except Exception:
                    pass
        except Exception:
            with open(os.path.join(state_dir, "relay-err.log"), "a") as err:
                err.write(traceback.format_exc() + "\n")
        finally:
            try:
                raw.close()
            except Exception:
                pass

    def accept_tls():
        while True:
            conn, _ = tls_srv.accept()
            threading.Thread(target=handle_tls, args=(conn,), daemon=True).start()

    def serve_static(conn, path_str):
        rel = path_str.lstrip("/") or "index.html"
        full = os.path.realpath(os.path.join(web_root, rel))
        if not web_root or not full.startswith(os.path.realpath(web_root)) or not os.path.isfile(full):
            conn.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            return
        body = open(full, "rb").read()
        ext = os.path.splitext(full)[1]
        ctype = MIME.get(ext) or mimetypes.guess_type(full)[0] or "application/octet-stream"
        hdr = (
            f"HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
        ).encode("utf-8")
        conn.sendall(hdr + body)

    def handle_ws(conn, headers):
        key = headers.get("sec-websocket-key", "")
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        conn.sendall(
            (
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\n"
                "Connection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
            ).encode("utf-8")
        )
        wlock = threading.Lock()
        first = ws_recv(conn, wlock)
        if first is None:
            return
        auth = json.loads(first)
        cred = auth.get("credential")
        reg = auth.get("register")

        # Host connection (uses its pre-saved host credential, not in browsers map).
        with mu:
            is_browser_cred = cred in state["browsers"] if cred else False

        if cred and not is_browser_cred:
            with mu:
                state["host"] = (conn, wlock)
                dev = state["host_device"]
            ws_send(conn, wlock, json.dumps({"kind": "authenticated", "device": dev, "credential": None}))
            try:
                while True:
                    msg = ws_recv(conn, wlock)
                    if msg is None:
                        break
                    wire = json.loads(msg)
                    kind = wire.get("kind")
                    if kind == "host":
                        with mu:
                            state["board"] = wire["board"]
                            state["invites"] = set(wire.get("invites", []))
                            state["devices"] = set(wire.get("devices", []))
                        ws_send(conn, wlock, json.dumps({"kind": "published"}))
                    elif kind == "collect":
                        ws_send(conn, wlock, json.dumps({"kind": "mail", "items": []}))
                    elif kind in ("welcome", "packet", "close"):
                        peer_id = wire.get("peer")
                        with mu:
                            target = state["peers"].get(peer_id)
                        if target:
                            psock, plock = target
                            if kind == "close":
                                try:
                                    psock.close()
                                except Exception:
                                    pass
                            else:
                                ws_send(psock, plock, json.dumps(wire))
            finally:
                with mu:
                    if state["host"] and state["host"][0] is conn:
                        state["host"] = None
            return

        # Browser connection.
        if reg:
            pub = reg["public"]
            dev_hex = device_id_of(pub)
            cred_hex = secrets.token_hex(32)
            info = {"device": dev_hex, "name": reg.get("name", "Phone"), "public": pub}
            with mu:
                state["browsers"][cred_hex] = info
            ws_send(conn, wlock, json.dumps({
                "kind": "authenticated",
                "device": dev_hex,
                "credential": cred_hex,
            }))
        elif is_browser_cred:
            with mu:
                info = state["browsers"][cred]
            ws_send(conn, wlock, json.dumps({
                "kind": "authenticated",
                "device": info["device"],
                "credential": None,
            }))
        else:
            ws_send(conn, wlock, json.dumps({"kind": "error", "code": "denied"}))
            return

        with mu:
            peer_id = f"peer-{state['next_peer']}"
            state["next_peer"] += 1
            state["peers"][peer_id] = (conn, wlock)

        try:
            while True:
                msg = ws_recv(conn, wlock)
                if msg is None:
                    break
                wire = json.loads(msg)
                kind = wire.get("kind")
                if kind == "pair":
                    with mu:
                        host = state["host"]
                    if host:
                        ws_send(host[0], host[1], json.dumps({
                            "kind": "peer",
                            "peer": peer_id,
                            "device": info["device"],
                            "name": info["name"],
                            "public": wire["public"],
                            "proof": wire["proof"],
                            "challenge": wire["challenge"],
                        }))
                elif kind == "connect":
                    with mu:
                        host = state["host"]
                    if host:
                        ws_send(host[0], host[1], json.dumps({
                            "kind": "peer",
                            "peer": peer_id,
                            "device": info["device"],
                            "name": info["name"],
                            "public": wire["public"],
                            "proof": None,
                            "challenge": wire["challenge"],
                        }))
                elif kind == "packet":
                    with mu:
                        host = state["host"]
                    if host:
                        ws_send(host[0], host[1], json.dumps({
                            "kind": "packet",
                            "peer": peer_id,
                            "record": wire["record"],
                        }))
                elif kind == "sync":
                    ws_send(conn, wlock, json.dumps({
                        "kind": "mailbox",
                        "board": wire["board"],
                        "items": [],
                    }))
                elif kind == "peek":
                    ws_send(conn, wlock, json.dumps({
                        "kind": "shelf",
                        "board": wire["board"],
                        "items": [],
                        "last": True,
                    }))
        finally:
            with mu:
                state["peers"].pop(peer_id, None)
                host = state["host"]
            if host:
                try:
                    ws_send(host[0], host[1], json.dumps({"kind": "gone", "peer": peer_id}))
                except Exception:
                    pass

    def handle_http(conn):
        try:
            raw = b""
            while b"\r\n\r\n" not in raw:
                chunk = conn.recv(4096)
                if not chunk:
                    return
                raw += chunk
            head = raw.split(b"\r\n\r\n", 1)[0].decode("utf-8", "replace")
            lines = head.split("\r\n")
            parts = lines[0].split(" ")
            path_str = parts[1].split("?", 1)[0] if len(parts) > 1 else "/"
            headers = {}
            for l in lines[1:]:
                if ":" in l:
                    k, v = l.split(":", 1)
                    headers[k.strip().lower()] = v.strip()
            if path_str == "/control" and headers.get("upgrade", "").lower() == "websocket":
                handle_ws(conn, headers)
            else:
                serve_static(conn, path_str)
        except Exception:
            pass
        finally:
            try:
                conn.close()
            except Exception:
                pass

    def accept_http():
        while True:
            conn, _ = http_srv.accept()
            threading.Thread(target=handle_http, args=(conn,), daemon=True).start()

    threading.Thread(target=accept_tls, daemon=True).start()
    threading.Thread(target=accept_http, daemon=True).start()

    info_path = os.path.join(state_dir, "relay-info.json")
    with open(info_path + ".tmp", "w") as out:
        json.dump({"tls_port": tls_port, "http_port": http_port, "origin": origin, "pin": pin}, out)
    os.replace(info_path + ".tmp", info_path)

    threading.Event().wait()


if __name__ == "__main__":
    main()
