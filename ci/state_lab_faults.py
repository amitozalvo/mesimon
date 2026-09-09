"""Local error injection for the real Claude CLI; no upstream proxy or model calls."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import time

ERRORS = {'fault-rate-limit': (429, 'rate_limit_error'), 'fault-auth': (401, 'authentication_error'),
          'fault-server': (500, 'api_error'), 'fault-model': (404, 'not_found_error')}


class FaultServer:
    def __init__(self, case, path):
        code, error = ERRORS[case]
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length','0')))
                # Deliberately never log request headers, bodies, or credentials.
                with path.open('a') as log:
                    log.write(json.dumps({'at_ms':time.time_ns()//1_000_000,'path':self.path,'status':code})+'\n')
                body=json.dumps({'type':'error','error':{'type':error,'message':'LAB_INJECTED_'+case}}).encode()
                self.send_response(code)
                self.send_header('Content-Type','application/json')
                self.send_header('Content-Length',str(len(body)))
                self.send_header('x-should-retry','false')
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *_args):
                pass
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.url='http://127.0.0.1:'+str(self.server.server_address[1])
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
