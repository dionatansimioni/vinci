// Points the download buttons at the newest AppImage on GitHub Releases. Without
// JavaScript (or if the API is unreachable) they keep linking to the releases page.
(function () {
  var api = "https://api.github.com/repos/dionatansimioni/vinci/releases/latest";
  fetch(api, { headers: { Accept: "application/vnd.github+json" } })
    .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
    .then(function (release) {
      var asset = (release.assets || []).find(function (a) {
        return /x86_64\.AppImage$/.test(a.name);
      });
      if (!asset) return;
      document.querySelectorAll("[data-download]").forEach(function (a) {
        a.href = asset.browser_download_url;
      });
      document.querySelectorAll("[data-version]").forEach(function (el) {
        el.textContent = release.tag_name.replace(/^v/, "");
      });
    })
    .catch(function () {});

  // Remember the language the visitor picked, so the switch link sticks.
  document.querySelectorAll("[data-lang]").forEach(function (a) {
    a.addEventListener("click", function () {
      try { localStorage.setItem("vinci-lang", a.getAttribute("data-lang")); } catch (e) {}
    });
  });
})();
