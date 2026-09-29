// Device keys and paired boards share one atomic IndexedDB record. Beside it,
// the last board each grant saw: titles, columns and agent states, never
// output, prompts or tool input. Revoke and forget drop it.
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
    // One board, or every remembered board when none is named.
    dropBoards: (board) =>
      run("readwrite", (s) =>
        s.delete(
          board
            ? IDBKeyRange.only(`board:${board}`)
            : IDBKeyRange.bound("board:", "board:￿"),
        ),
      ),
  };
}
