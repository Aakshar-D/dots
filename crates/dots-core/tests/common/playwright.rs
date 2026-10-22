//! Results of `@playwright/mcp@0.0.83` as it wrote them on 2026-10-21, driving Microsoft Edge
//! 155 over `--cdp-endpoint` against a local test page (the browser plan, "What was
//! measured"), and Edge's own `/json/list`. The browser's tests read these; a new pinned
//! version comes with new captures (browser spec section 11).

/// The addresses the captures were taken on: one local test server per session.
pub const CAPTURED_URLS: [&str; 3] = [
    "http://127.0.0.1:54428/cart",
    "http://127.0.0.1:63843/cart",
    "http://127.0.0.1:64868/cart",
];

/// `text` with every captured page address replaced by `url`.
pub fn on(text: &str, url: &str) -> String {
    CAPTURED_URLS
        .iter()
        .fold(text.to_string(), |text, captured| {
            text.replace(captured, url)
        })
}

/// `browser_snapshot` with no argument.
pub const SNAPSHOT: &str = r#"### Open tabs
- 0: (current) [Shop "Test"](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
### Page
- Page URL: http://127.0.0.1:54428/cart
- Page Title: Shop "Test"
### Snapshot
```yaml
- generic [ref=e1]:
  - heading "Your cart" [level=1] [ref=e2]
  - navigation [ref=e3]:
    - list [ref=e4]:
      - listitem [ref=e5]:
        - link "Home" [ref=e6] [cursor=pointer]:
          - /url: /a
      - listitem [ref=e7]:
        - link "My orders" [ref=e8] [cursor=pointer]:
          - /url: /orders
  - generic [ref=e9]:
    - generic [ref=e10]:
      - text: Search
      - textbox "Search" [ref=e11]
    - generic [ref=e12]:
      - text: Password
      - textbox "Password" [ref=e13]
    - generic [ref=e14]:
      - text: One-time code
      - textbox "One-time code" [ref=e15]
    - button "Pay now" [ref=e16]
    - button "Say \"hi\"" [ref=e17]
    - button "Delete account" [ref=e18]
    - combobox "Size" [ref=e19]:
      - option "Small" [selected]
      - option "Large"
    - checkbox "Remember me" [ref=e20]
  - link "Download statement" [ref=e21] [cursor=pointer]:
    - /url: /file.txt
```"#;

/// `browser_navigate` to the cart page (default snapshot mode: the snapshot went to a file).
pub const NAVIGATE: &str = r#"### Ran Playwright code
```js
await page.goto('http://127.0.0.1:54428/cart');
```
### Open tabs
- 0: (current) [Shop "Test"](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
### Page
- Page URL: http://127.0.0.1:54428/cart
- Page Title: Shop "Test"
### Snapshot
- [Snapshot](downloads\page-2026-10-21T12-16-34-013Z.yml)
### Events
- New console entries: downloads\console-2026-10-21T12-16-33-922Z.log#L1"#;

/// `browser_tabs` with `action: "new"` and a `data:` URL.
pub const TABS_NEW: &str = r#"### Result
- 0: [paid](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
- 2: (current) [](data:text/html,<button>Second tab</button>)
### Ran Playwright code
```js
await page.goto('data:text/html,<button>Second tab</button>');
```
### Open tabs
- 0: [paid](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
- 2: (current) [](data:text/html,<button>Second tab</button>)
### Page
- Page URL: data:text/html,<button>Second tab</button>
### Snapshot
- [Snapshot](downloads\page-2026-10-21T12-16-35-323Z.yml)"#;

/// `browser_tabs` with `action: "select"`, `index: 0`, after `TABS_NEW`.
pub const TABS_SELECT: &str = r#"### Result
- 0: (current) [paid](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
- 2: [](data:text/html,<button>Second tab</button>)"#;

/// `browser_snapshot` with `filename: "snap.md"`.
pub const SNAPSHOT_TO_FILE: &str = r#"### Open tabs
- 0: (current) [paid](http://127.0.0.1:54428/cart)
- 1: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
- 2: [](data:text/html,<button>Second tab</button>)
### Page
- Page URL: http://127.0.0.1:54428/cart
- Page Title: paid
### Snapshot
- [Snapshot](./snap.md)"#;

/// `browser_snapshot` with `depth: 1` (another session, another cart page).
pub const SNAPSHOT_DEPTH_1: &str = r#"### Open tabs
- 0: (current) [Shop](http://127.0.0.1:63843/cart)
- 1: [](about:blank)
- 2: [We are now syncing your browsing data across all your devices](edge://sync-confirmation-dialog/)
### Page
- Page URL: http://127.0.0.1:63843/cart
- Page Title: Shop
### Snapshot
```yaml
- generic [ref=e1]:
  - heading "Your cart" [level=1] [ref=e2]
  - generic [ref=e3]
  - link "Download statement" [ref=e11] [cursor=pointer]:
    - /url: /file.txt
  - link "Open other" [ref=e12] [cursor=pointer]:
    - /url: /other
```"#;

/// `browser_find` with `text: "pay"` on that page.
pub const FIND: &str = r#"### Result
Found 1 match for "pay":

- generic [ref=e1]:
  - generic [ref=e3]:
    - generic [ref=e6]:
      - text: Password
      - textbox "Password" [ref=e7]
    - button "Pay now" [ref=e8]
    - button "Delete account" [ref=e9]
    - checkbox "Remember me" [ref=e10]
  - link "Download statement" [ref=e11] [cursor=pointer]:"#;

/// `browser_click` on a button, with `--snapshot-mode none` (one tab: no tab list).
pub const CLICK: &str = r#"### Ran Playwright code
```js
await page.getByRole('button', { name: 'Pay now' }).click();
```
### Page
- Page URL: http://127.0.0.1:64868/cart
- Page Title: paid
### Snapshot
- [Snapshot](downloads\page-2026-10-21T12-22-45-097Z.yml)"#;

/// `browser_click` on a button that opens `confirm("Really delete?")`.
pub const CLICK_DIALOG: &str = r#"### Ran Playwright code
```js
await page.getByRole('button', { name: 'Delete account' }).click();
```
### Open tabs
- 0: (current) [](http://127.0.0.1:64868/cart)
- 1: [Downloads](edge://downloads-hub/)
### Page
- Page URL: http://127.0.0.1:64868/cart
### Modal state
- ["confirm" dialog with message "Really delete?"]: can be handled by browser_handle_dialog
### Snapshot
- [Snapshot](downloads\page-2026-10-21T12-22-48-000Z.yml)"#;

/// `browser_click` with `target: "e999"` (an `isError` result).
pub const UNKNOWN_REF: &str = r#"### Error
Error: Ref e999 not found in the current page snapshot. Try capturing new snapshot."#;

/// The text part of `browser_take_screenshot` (an image part follows it).
pub const SCREENSHOT: &str = r#"### Result
- [Screenshot of viewport](downloads\page-2026-10-21T12-16-35-575Z.png)
### Ran Playwright code
```js
// Screenshot viewport and save it as downloads\page-2026-10-21T12-16-35-575Z.png
await page.screenshot({
  path: 'downloads\\page-2026-10-21T12-16-35-575Z.png',
  scale: 'css',
  type: 'png'
});
```"#;

/// Edge's `GET /json/list`, three of its targets: an Edge page, an extension's background
/// page, and the cart page.
pub const JSON_LIST: &str = r#"[ {
   "description": "",
   "devtoolsFrontendUrl": "https://aka.ms/docs-landing-page/serve_rev/@50f24ef2aa5369a074b8888b7db82d9c0652d642/inspector.html?ws=127.0.0.1:9347/devtools/page/3D8B009E507F2FA23B827CEDF5E33BFD",
   "id": "3D8B009E507F2FA23B827CEDF5E33BFD",
   "title": "We are now syncing your browsing data across all your devices",
   "type": "page",
   "url": "edge://sync-confirmation-dialog/",
   "webSocketDebuggerUrl": "ws://127.0.0.1:9347/devtools/page/3D8B009E507F2FA23B827CEDF5E33BFD"
}, {
   "description": "",
   "devtoolsFrontendUrl": "https://aka.ms/docs-landing-page/serve_rev/@50f24ef2aa5369a074b8888b7db82d9c0652d642/inspector.html?ws=127.0.0.1:9347/devtools/page/7F9AC21EBE933C2E9EB11984991D9496",
   "id": "7F9AC21EBE933C2E9EB11984991D9496",
   "title": "Microsoft Voices",
   "type": "background_page",
   "url": "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold/_generated_background_page.html",
   "webSocketDebuggerUrl": "ws://127.0.0.1:9347/devtools/page/7F9AC21EBE933C2E9EB11984991D9496"
}, {
   "description": "",
   "devtoolsFrontendUrl": "https://aka.ms/docs-landing-page/serve_rev/@50f24ef2aa5369a074b8888b7db82d9c0652d642/inspector.html?ws=127.0.0.1:9347/devtools/page/3764F99E34F4B738CF9C0B69FF65BAF7",
   "id": "3764F99E34F4B738CF9C0B69FF65BAF7",
   "title": "Shop &quot;Test&quot;",
   "type": "page",
   "url": "http://127.0.0.1:54428/cart",
   "webSocketDebuggerUrl": "ws://127.0.0.1:9347/devtools/page/3764F99E34F4B738CF9C0B69FF65BAF7"
} ]"#;
