// The download buttons link to the latest GitHub release, so they never go stale.

// Remember the language the visitor picked, so the switch link sticks.
document.querySelectorAll("[data-lang]").forEach(function (a) {
  a.addEventListener("click", function () {
    try { localStorage.setItem("vinci-lang", a.getAttribute("data-lang")); } catch (e) {}
  });
});
