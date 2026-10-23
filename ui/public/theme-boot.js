// Shows the chosen theme before the first paint (deep-teal spec section 3): index.html loads
// this file as a classic script in its <head>, so it runs before the page is drawn. It is a
// file, not an inline script, because the app's content security policy only runs scripts of
// the app itself. `src/lib/theme.ts` holds the same rule; `src/lib/theme.test.tsx` runs this
// file against it.
function dotsBootTheme(storage, prefersDark, root) {
  var choice = null;
  try {
    choice = storage.getItem("dots.theme");
  } catch (e) {
    // Storage that cannot be read counts as System.
  }
  var dark = choice === "dark" || (choice !== "light" && prefersDark);
  root.classList.toggle("dark", dark);
  root.style.colorScheme = dark ? "dark" : "light";
}

(function () {
  var storage = null;
  try {
    storage = window.localStorage;
  } catch (e) {
    // Blocked storage: System.
  }
  var prefersDark =
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches;
  dotsBootTheme(storage, prefersDark, document.documentElement);
})();
