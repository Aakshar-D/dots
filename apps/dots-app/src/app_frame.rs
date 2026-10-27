//! The page an app's screen runs in (app library spec section 5, "Screen"). The `dots-app`
//! protocol serves the files below `ui/` of an installed app's version
//! (`/app/<name>/<version>/<path>`) or of a review card's preview (`/preview/<id>/<path>`), and
//! nothing else. Each answer carries a CSP that lets the page load only from its own folder and
//! connect nowhere, and `index.html` gets Dots' client for the bridge (`app_client.js`) at the
//! top. The UI frames the page with `sandbox="allow-scripts"`, so it runs in an opaque origin
//! and never sees Tauri's IPC.

use dots_core::{Error, Runtime};
use tauri::http::{header, Method, Request, Response, StatusCode};

/// The protocol's name.
pub const SCHEME: &str = "dots-app";
/// Where WebView2 serves the protocol on Windows. The UI frames pages from it
/// (`ui/src/lib/app-bridge.ts`), and Dots' own CSP lets it be framed (`tauri.conf.json`).
pub const ORIGIN: &str = "http://dots-app.localhost";
/// The client the page talks to Dots through.
pub const CLIENT: &str = include_str!("app_client.js");

/// What a request asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A file of the app `name` at `version`.
    App {
        name: String,
        version: u32,
        path: String,
    },
    /// A file of the preview `id`.
    Preview { id: String, path: String },
}

impl Target {
    /// The path below `ui/`.
    pub fn path(&self) -> &str {
        match self {
            Target::App { path, .. } | Target::Preview { path, .. } => path,
        }
    }

    /// The folder the page may load from: its own `ui/` files.
    pub fn base(&self) -> String {
        match self {
            Target::App { name, version, .. } => format!("{ORIGIN}/app/{name}/{version}/"),
            Target::Preview { id, .. } => format!("{ORIGIN}/preview/{id}/"),
        }
    }
}

/// Decodes `%XX` escapes; `None` for a broken one or bytes that are not UTF-8.
fn decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = segment.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The target of the request path `path`; `None` for anything else. An empty file path is
/// `index.html`. The files are looked up by name in the app's own list, so no path reaches
/// past it; a `..`, `.` or empty segment is refused all the same.
pub fn parse(path: &str) -> Option<Target> {
    let (kind, rest) = path.strip_prefix('/')?.split_once('/')?;
    let (head, file) = match kind {
        "app" => {
            let mut it = rest.splitn(3, '/');
            let name = it.next()?.to_string();
            let digits = it.next()?;
            // Canonical decimal only: `+3` and `03` would name a second address for version 3.
            let canonical = !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits == "0" || !digits.starts_with('0'));
            if !canonical {
                return None;
            }
            let version: u32 = digits.parse().ok()?;
            // A page at `/app/<name>/<version>` has no folder of its own: its relative URLs would
            // resolve past it. So a slash must follow the version, even when nothing comes after.
            (Some((name, version)), it.next()?)
        }
        "preview" => (None, rest),
        _ => return None,
    };
    let (id, file) = match head {
        Some(_) => (None, file),
        None => {
            let (id, file) = file.split_once('/').unwrap_or((file, ""));
            (Some(id.to_string()), file)
        }
    };
    let file = if file.is_empty() { "index.html" } else { file };
    let mut segments = Vec::new();
    for segment in file.split('/') {
        let segment = decode(segment)?;
        // A decoded `/` (`%2F`) or NUL (`%00`) names a file the path split did not see.
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains(['\\', '/', '\0'])
        {
            return None;
        }
        segments.push(segment);
    }
    let path = segments.join("/");
    Some(match (head, id) {
        (Some((name, version)), _) => Target::App {
            name,
            version,
            path,
        },
        (None, Some(id)) if !id.is_empty() => Target::Preview { id, path },
        _ => return None,
    })
}

/// The page's CSP (spec section 5): scripts, styles, images and fonts from its own folder (and
/// inline), images also as `data:`; no connection, frame, form, or base URL.
pub fn csp(base: &str) -> String {
    format!(
        "default-src 'none'; script-src 'unsafe-inline' {base}; style-src 'unsafe-inline' {base}; \
         img-src data: {base}; font-src {base}; connect-src 'none'; frame-src 'none'; \
         form-action 'none'; base-uri 'none'"
    )
}

/// The type of the file `path`, by its extension.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `html` with the client at its top: after a leading `<!doctype …>` (which must stay first, or
/// the page renders in quirks mode), else before everything.
pub fn with_client(html: &[u8]) -> Vec<u8> {
    let script = format!("<script>{CLIENT}</script>");
    let text = String::from_utf8_lossy(html);
    let lead = text.len()
        - text
            .trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n'])
            .len();
    let at = if text[lead..].to_ascii_lowercase().starts_with("<!doctype") {
        text[lead..].find('>').map_or(lead, |end| lead + end + 1)
    } else {
        lead
    };
    let mut out = String::with_capacity(text.len() + script.len());
    out.push_str(&text[..at]);
    out.push_str(&script);
    out.push_str(&text[at..]);
    out.into_bytes()
}

fn plain(status: StatusCode, text: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
        .body(text.as_bytes().to_vec())
        .expect("a plain response")
}

/// The answer for `target` once its file was looked up.
pub fn response(target: &Target, found: dots_core::Result<Vec<u8>>) -> Response<Vec<u8>> {
    let bytes = match found {
        Ok(bytes) => bytes,
        Err(Error::NotFound(_)) => return plain(StatusCode::NOT_FOUND, "not found"),
        Err(e) => {
            tracing::warn!("app file could not be read: {e}");
            return plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the file could not be read",
            );
        }
    };
    let path = target.path();
    let body = if path == "index.html" {
        with_client(&bytes)
    } else {
        bytes
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::CONTENT_SECURITY_POLICY, csp(&target.base()))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        // The frame's origin is opaque, so fonts and module scripts load in CORS mode.
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store")
        .header("Referrer-Policy", "no-referrer")
        .body(body)
        .expect("a file response")
}

/// Answers a request of the protocol: only `GET` of a file an app or a preview has.
pub async fn respond(rt: Option<&Runtime>, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() != Method::GET {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "only GET");
    }
    let Some(target) = parse(request.uri().path()) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let Some(rt) = rt else {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "Dots did not start");
    };
    let found = match &target {
        Target::App {
            name,
            version,
            path,
        } => rt.apps().ui_file(name, *version, path).await,
        Target::Preview { id, path } => rt.apps().preview_file(id, path),
    };
    response(&target, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, version: u32, path: &str) -> Option<Target> {
        Some(Target::App {
            name: name.into(),
            version,
            path: path.into(),
        })
    }

    #[test]
    fn a_path_names_an_app_version_or_a_preview_and_a_file_below_ui() {
        assert_eq!(
            parse("/app/expenses/3/index.html"),
            app("expenses", 3, "index.html")
        );
        assert_eq!(parse("/app/expenses/3/"), app("expenses", 3, "index.html"));
        assert_eq!(
            parse("/app/expenses/3/img/my%20logo.png"),
            app("expenses", 3, "img/my logo.png")
        );
        assert_eq!(
            parse("/preview/01J0PREV/app.js"),
            Some(Target::Preview {
                id: "01J0PREV".into(),
                path: "app.js".into()
            })
        );
        for refused in [
            "/",
            "/app/expenses",
            "/app/expenses/x/index.html",
            "/app/expenses/3",
            "/app/expenses/+3/index.html",
            "/app/expenses/03/index.html",
            "/app/expenses/3/../logic.js",
            "/app/expenses/3/%2e%2e/logic.js",
            "/app/expenses/3/..%2Flogic.js",
            "/app/expenses/3/a//b.js",
            "/app/expenses/3/a%5cb.js",
            "/app/expenses/3/a%00.js",
            "/app/expenses/3/%zz",
            "/preview/",
            "/other/x",
        ] {
            assert_eq!(parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_page_may_load_only_its_own_files_and_reach_nothing() {
        let target = app("expenses", 3, "index.html").unwrap();
        let policy = csp(&target.base());
        assert!(policy.starts_with("default-src 'none'; "), "{policy}");
        for part in [
            "script-src 'unsafe-inline' http://dots-app.localhost/app/expenses/3/;",
            "style-src 'unsafe-inline' http://dots-app.localhost/app/expenses/3/;",
            "img-src data: http://dots-app.localhost/app/expenses/3/;",
            "connect-src 'none';",
            "frame-src 'none';",
            "form-action 'none';",
            "base-uri 'none'",
        ] {
            assert!(policy.contains(part), "{part} in {policy}");
        }
        assert!(!policy.contains("'self'"), "{policy}");
        assert!(!policy.contains("ipc:"), "{policy}");
    }

    #[test]
    fn index_html_gets_the_client_after_its_doctype() {
        let page = String::from_utf8(with_client(b"<!DOCTYPE html>\n<title>E</title>")).unwrap();
        assert!(
            page.starts_with("<!DOCTYPE html><script>"),
            "{}",
            &page[..40]
        );
        assert!(page.ends_with("</script>\n<title>E</title>"));
        let bare = String::from_utf8(with_client(b"<div id=app></div>")).unwrap();
        assert!(
            bare.starts_with("<script>// Dots' client"),
            "{}",
            &bare[..40]
        );
        assert!(bare.ends_with("</script><div id=app></div>"));
    }

    #[test]
    fn a_file_is_served_with_its_type_and_the_pages_csp() {
        let target = app("expenses", 3, "index.html").unwrap();
        let page = response(&target, Ok(b"<!doctype html><p>hi".to_vec()));
        assert_eq!(page.status(), StatusCode::OK);
        let headers = page.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            csp(&target.base()).as_str()
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert!(String::from_utf8_lossy(page.body()).contains("window, \"dots\""));

        let script = app("expenses", 3, "app.js").unwrap();
        let js = response(&script, Ok(b"dots.call('x')".to_vec()));
        assert_eq!(
            js.headers()[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            js.body(),
            b"dots.call('x')",
            "only index.html gets the client"
        );

        let missing = response(&script, Err(Error::NotFound("ui/app.js".into())));
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(content_type("ui/font.WOFF2"), "font/woff2");
        assert_eq!(content_type("data.bin"), "application/octet-stream");
    }

    #[test]
    fn a_failed_read_answers_a_fixed_text_and_no_detail() {
        let target = app("expenses", 3, "app.js").unwrap();
        let failed = response(
            &target,
            Err(Error::Io(std::io::Error::other("C:\\secret\\path"))),
        );
        assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(failed.body(), b"the file could not be read");
    }

    #[tokio::test]
    async fn only_get_of_a_known_path_is_answered() {
        let post = Request::builder()
            .method(Method::POST)
            .uri("http://dots-app.localhost/app/expenses/1/index.html")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            respond(None, &post).await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let other = Request::builder()
            .uri("http://dots-app.localhost/secrets")
            .body(Vec::new())
            .unwrap();
        assert_eq!(respond(None, &other).await.status(), StatusCode::NOT_FOUND);
        let early = Request::builder()
            .uri("http://dots-app.localhost/app/expenses/1/index.html")
            .body(Vec::new())
            .unwrap();
        assert_eq!(
            respond(None, &early).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
