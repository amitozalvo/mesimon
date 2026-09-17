// Device keys and paired boards share one atomic IndexedDB record.
export async function openIdentity() {
  const db = await new Promise((resolve, reject) => {
    const req = indexedDB.open("mesophon", 1);
    req.onupgradeneeded = () => req.result.createObjectStore("device");
    req.onerror = () => reject(req.error);
    req.onsuccess = () => resolve(req.result);
  });
  return {
    read: () =>
      new Promise((resolve, reject) => {
        const req = db
          .transaction("device")
          .objectStore("device")
          .get("identity");
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      }),
    save: (identity) =>
      new Promise((resolve, reject) => {
        const tx = db.transaction("device", "readwrite");
        tx.objectStore("device").put(identity, "identity");
        tx.oncomplete = resolve;
        tx.onerror = tx.onabort = () => reject(tx.error);
      }),
  };
}
