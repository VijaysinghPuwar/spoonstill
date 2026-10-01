// Run the window's script once against a stand-in page (D-194).
//
// `node --check` proves app.js parses. It does not prove app.js *runs*: a
// handler wired to a function that no longer exists throws a ReferenceError
// on the first line that names it, and everything after that line never
// happens — the window opens on its own title bar and nothing else. That is
// exactly how v0.1.15's successor first came out of an edit, and every other
// check in this tree passed it.
//
// The stand-in answers every DOM call with something callable, and every
// command with nothing, so this asserts one property only: the top level of
// the script reaches its last line.
const anything = () =>
  new Proxy(function () {}, {
    get: (_, key) =>
      key === Symbol.toPrimitive ? () => "" : key === "then" ? undefined : anything(),
    apply: () => anything(),
    construct: () => anything(),
    set: () => true,
  });

globalThis.window = {
  __TAURI__: {
    core: { invoke: async () => [], Channel: class {}, convertFileSrc: (path) => path },
    event: { listen: async () => () => {} },
  },
  devicePixelRatio: 2,
  matchMedia: () => ({ matches: false, addEventListener() {} }),
};
globalThis.document = anything();
globalThis.localStorage = { getItem: () => null, setItem() {} };
Object.defineProperty(globalThis, "navigator", {
  value: { userAgent: "Macintosh", platform: "MacIntel" },
  configurable: true,
});
globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
globalThis.innerHeight = 800;
globalThis.CSS = { escape: (text) => text };
globalThis.getComputedStyle = () => ({ lineHeight: "20" });

import { pathToFileURL } from "node:url";
import { resolve } from "node:path";

const script = process.argv[2];
try {
  // `pathToFileURL`, not string concatenation: a Windows path is not a URL.
  await import(pathToFileURL(resolve(script)).href);
} catch (error) {
  console.error(`  ${script} stops while starting: ${error}`);
  process.exit(1);
}
