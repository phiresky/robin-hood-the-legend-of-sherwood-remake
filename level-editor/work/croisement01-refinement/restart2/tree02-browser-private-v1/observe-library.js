(() => {
  const original = window.fetch.bind(window);
  const stable = value => Array.isArray(value) ? value.map(stable) : value && typeof value === 'object'
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, stable(value[key])])) : value;
  const digest = async bytes => [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(x=>x.toString(16).padStart(2,'0')).join('');
  window.observedPublicationResponses = [];
  window.observedPublicationPending = [];
  window.fetch = async (...args) => {
    const response = await original(...args);
    const request = args[0] instanceof Request ? args[0].url : String(args[0]);
    const path = new URL(request,location.href).pathname;
    const index = path === '/library/3d-assets/index.json';
    const selected = /^\/library\/3d-assets\/croisement01\/croisement01-(tree-02|group-005)\//.test(path);
    if (response.ok && (index || selected)) {
      const copy = response.clone();
      const pending = (async () => {
        const bytes = await copy.arrayBuffer();
        const row = { path, sha256: await digest(bytes), bytes: bytes.byteLength };
        if (index) {
          const value = JSON.parse(new TextDecoder().decode(bytes));
          const subset = value.assets.filter(asset=>asset.source_map?.toLowerCase()==='croisement01').sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0);
          row.c02_sha256 = await digest(new TextEncoder().encode(JSON.stringify(stable(subset))));
          row.c02_count = subset.length;
        }
        window.observedPublicationResponses.push(row);
      })();
      window.observedPublicationPending.push(pending);
    }
    return response;
  };
})();
