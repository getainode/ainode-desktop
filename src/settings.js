// Settings screen: two addresses, a Test button for each, Save.
(function () {
  const { invoke } = window.__TAURI__.core;

  const primary = document.getElementById("primary");
  const alternate = document.getElementById("alternate");
  const apiKey = document.getElementById("api-key");
  const showKey = document.getElementById("show-key");
  const error = document.getElementById("error");
  const save = document.getElementById("save");
  const cancel = document.getElementById("cancel");

  function setResult(id, cls, text) {
    const el = document.getElementById("result-" + id);
    el.className = "result " + cls;
    el.textContent = text;
  }

  async function test(id) {
    const input = document.getElementById(id);
    const button = document.getElementById("test-" + id);
    const value = input.value.trim();
    if (!value) {
      setResult(id, "bad", id === "alternate" ? "Nothing to test." : "Enter an address first.");
      return;
    }
    button.disabled = true;
    setResult(id, "busy", "Testing " + value + "...");
    try {
      const r = await invoke("test_address", { address: value });
      const model = r.model ? " · " + r.model.split("/").pop() : "";
      // The lock says the test reached it over https, which is the transport the
      // app will use for it from now on. Never a claim about what it could do.
      const lock = r.tls ? "\ud83d\udd12 " : "";
      setResult(id, "ok", lock + r.node_name + " · AINode " + r.version + model);
      // Not r.address: that is the upgraded https address, and this box holds the
      // plain one the fleet list is keyed on.
      input.value = value;
    } catch (e) {
      setResult(id, "bad", "No answer from " + value + ": " + String(e));
    } finally {
      button.disabled = false;
    }
  }

  async function doSave() {
    error.textContent = "";
    save.disabled = true;
    try {
      await invoke("save_settings", {
        primary: primary.value,
        alternate: alternate.value,
        apiKey: apiKey.value,
      });
      await invoke("close_self");
    } catch (e) {
      error.textContent = String(e);
      save.disabled = false;
    }
  }

  document.getElementById("test-primary").addEventListener("click", () => test("primary"));
  document.getElementById("test-alternate").addEventListener("click", () => test("alternate"));
  save.addEventListener("click", doSave);
  cancel.addEventListener("click", () => invoke("close_self"));

  showKey.addEventListener("click", () => {
    const hidden = apiKey.type === "password";
    apiKey.type = hidden ? "text" : "password";
    showKey.textContent = hidden ? "Hide" : "Show";
  });

  for (const input of [primary, alternate, apiKey]) {
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        doSave();
      } else if (ev.key === "Escape") {
        invoke("close_self");
      }
    });
    input.addEventListener("input", () => {
      error.textContent = "";
      setResult(input.id, "", "");
    });
  }

  // Which node is answering, and how many fallbacks are saved. Read-only: the
  // fleet list comes from the nodes themselves, so there is nothing to edit.
  const serving = document.getElementById("serving");

  function fleetLine(count) {
    if (!count) return "";
    return count === 1
      ? " · 1 other node of the fleet saved as a fallback"
      : " · " + count + " other nodes of the fleet saved as fallbacks";
  }

  async function refreshServing() {
    try {
      const s = await invoke("get_status_view");
      if (!s.configured) {
        serving.className = "result";
        serving.textContent = "";
        return;
      }
      if (!s.master) {
        serving.className = "result bad";
        // The actionable reason wins: "this node wants an API key" is something
        // to fix in the box below, and "nothing answered" is not.
        serving.textContent =
          (s.needs_attention || s.last_error || "Not connected") + fleetLine(s.fleet_count);
        return;
      }
      const who = s.serving_via || "Connected to " + (s.serving_name || s.master);
      const lock = s.tls ? "\ud83d\udd12 " : "";
      serving.className = "result ok";
      serving.textContent =
        lock + who + " (" + (s.tls ? "https" : "http") + "://" + s.master + ")" +
        fleetLine(s.fleet_count);
      // A node that is only reached over http and holds a key is worth one line.
      if (!s.tls && s.has_api_key) {
        serving.textContent +=
          " · the key travels in clear text: run `ainode tls enable --tailscale` on the node";
      }
    } catch (e) {
      serving.className = "result";
      serving.textContent = "";
    }
  }

  invoke("get_settings").then((s) => {
    primary.value = s.primary || "";
    alternate.value = s.alternate || "";
    apiKey.value = s.api_key || "";
    primary.focus();
  });
  refreshServing();
  setInterval(refreshServing, 2000);
})();
