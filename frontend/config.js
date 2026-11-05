// Website settings. Edit before deploying.
window.WOWTRACKER = {
  // Where the backend's read-only API lives.
  // "api" works when the same site proxies /api to the backend
  // (Caddy reverse_proxy, or the rewrite in vercel.json).
  // A full URL such as "https://wowtracker-api.osrs.dk/api" calls the backend
  // directly; then add this site's origin to cors_origins in the backend config.
  apiBase: "api",

  // How often to check for new data. The game only saves on /reload or
  // logout, so there is no point polling very fast.
  pollSeconds: 15,

  title: "WoW Tracker",
};
