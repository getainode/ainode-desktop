// About: version from the app, link to the product site.
(function () {
  const { invoke } = window.__TAURI__.core;
  invoke("app_info").then((info) => {
    document.getElementById("version").textContent = info.version;
  });
  document.getElementById("site").addEventListener("click", () => invoke("open_site"));
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") {
      invoke("close_self");
    }
  });
})();
