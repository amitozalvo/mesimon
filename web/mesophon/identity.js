// Device keys and paired boards share one atomic IndexedDB record. Beside it,
// per grant: the last board it saw (titles, columns, tags and agent states,
// never output, prompts or tool input) and the tickets this browser sent it.
// Revoke and forget drop both.
const remembered = ["board:", "sent:"];

export async function openIdentity() {
  const db = await new Promise((resolve, reject) => {
    const req = indexedDB.open("mesophon", 1);
    req.onupgradeneeded = () => req.result.createObjectStore("device");
    req.onerror = () => reject(req.error);
    req.onsuccess = () => resolve(req.result);
  });
  const run = (mode, act) =>
    new Promise((resolve, reject) => {
      const tx = db.transaction("device", mode);
      const req = act(tx.objectStore("device"));
      tx.oncomplete = () => resolve(req.result);
      tx.onerror = tx.onabort = () => reject(tx.error);
    });
  return {
    read: () => run("readonly", (s) => s.get("identity")),
    save: (identity) => run("readwrite", (s) => s.put(identity, "identity")),
    readBoard: (board) => run("readonly", (s) => s.get(`board:${board}`)),
    saveBoard: (board, value) =>
      run("readwrite", (s) => s.put(value, `board:${board}`)),
    readSent: (board) => run("readonly", (s) => s.get(`sent:${board}`)),
    saveSent: (board, value) =>
      run("readwrite", (s) => s.put(value, `sent:${board}`)),
    // One board's memory, or every board's when none is named.
    dropBoards: (board) =>
      run("readwrite", (s) => {
        let last;
        for (const prefix of remembered)
          last = s.delete(
            board
              ? IDBKeyRange.only(`${prefix}${board}`)
              : IDBKeyRange.bound(prefix, `${prefix}￿`),
          );
        return last;
      }),
  };
}
