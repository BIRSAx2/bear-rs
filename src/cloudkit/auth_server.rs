use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use tiny_http::{Header, Method, Response, Server};

use super::client::API_TOKEN;
use crate::verbose;

const PREFERRED_PORT: u16 = 19222;
const TIMEOUT_SECS: u64 = 120;

/// Opens a browser-based Apple sign-in page and waits for the `ckWebAuthToken`
/// to be POSTed back, then returns it.
pub fn acquire_token() -> Result<String> {
    let server = Server::http(format!("127.0.0.1:{PREFERRED_PORT}"))
        .or_else(|_| Server::http("127.0.0.1:0"))
        .map_err(|e| anyhow!("failed to start auth server: {e}"))?;

    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(PREFERRED_PORT);
    eprintln!("Opening http://localhost:{port}/ in Safari...");
    eprintln!("Sign in with your Apple ID. The window will close automatically.");
    verbose::eprintln(1, format!("[auth] listening on 127.0.0.1:{port}"));
    open_browser(port);

    let token: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let deadline = std::time::Instant::now() + Duration::from_secs(TIMEOUT_SECS);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!("auth timed out after {TIMEOUT_SECS}s"));
        }

        match server.recv_timeout(Duration::from_millis(200))? {
            None => continue,
            Some(request) => {
                let got_token = handle_request(request, port, &token)?;
                if got_token {
                    return token
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or_else(|| anyhow!("token was set but is empty"));
                }
            }
        }
    }
}

fn handle_request(
    mut request: tiny_http::Request,
    port: u16,
    token: &Arc<Mutex<Option<String>>>,
) -> Result<bool> {
    let raw_url = request.url().to_string();
    let path = raw_url.split('?').next().unwrap_or("/").to_string();
    if verbose::enabled(3) {
        verbose::eprintln(
            3,
            format!(
                "[auth] {} {}",
                request.method().as_str(),
                redact_auth_url(&raw_url)
            ),
        );
    }

    match (request.method(), path.as_str()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            if let Some(t) = extract_token_from_url(&raw_url) {
                verbose::eprintln(2, "[auth] captured ckWebAuthToken from request URL");
                *token.lock().unwrap() = Some(t);
                let response = Response::from_data(success_html().as_bytes().to_vec())
                    .with_header(content_type("text/html; charset=utf-8"));
                request.respond(response)?;
                return Ok(true);
            }

            let html = auth_html(port);
            let response = Response::from_data(html.into_bytes())
                .with_header(content_type("text/html; charset=utf-8"));
            request.respond(response)?;
        }

        (Method::Get, "/callback") => {
            if let Some(t) = extract_token_from_url(&raw_url) {
                verbose::eprintln(2, "[auth] captured ckWebAuthToken from callback URL");
                *token.lock().unwrap() = Some(t);
                let response = Response::from_data(success_html().as_bytes().to_vec())
                    .with_header(content_type("text/html; charset=utf-8"));
                request.respond(response)?;
                return Ok(true);
            }

            let response = Response::from_data(callback_html(port).into_bytes())
                .with_header(content_type("text/html; charset=utf-8"));
            request.respond(response)?;
        }

        (Method::Post, "/callback") => {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap_or(0);
            if verbose::enabled(3) {
                verbose::eprintln(
                    3,
                    format!("[auth] callback body: {}", redact_auth_body(&body)),
                );
            }

            let cors = Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap();
            if let Some(t) = extract_token_from_json(&body) {
                verbose::eprintln(2, "[auth] captured ckWebAuthToken from callback JSON");
                *token.lock().unwrap() = Some(t);
                let response = Response::from_data(b"{\"status\":\"ok\"}".to_vec())
                    .with_header(content_type("application/json"))
                    .with_header(cors);
                request.respond(response)?;
                return Ok(true);
            } else {
                let response = Response::from_data(b"{\"error\":\"Missing token\"}".to_vec())
                    .with_status_code(400)
                    .with_header(content_type("application/json"))
                    .with_header(cors);
                request.respond(response)?;
            }
        }

        (Method::Options, _) => {
            let response = Response::empty(204)
                .with_header(Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap())
                .with_header(
                    Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
                        .unwrap(),
                )
                .with_header(
                    Header::from_bytes("Access-Control-Allow-Headers", "Content-Type").unwrap(),
                );
            request.respond(response)?;
        }

        (Method::Get, "/health") => {
            let response = Response::from_data(b"{\"status\":\"ok\"}".to_vec())
                .with_header(content_type("application/json"));
            request.respond(response)?;
        }

        (Method::Get, "/favicon.ico") => {
            request.respond(Response::empty(204))?;
        }

        _ => {
            request.respond(Response::empty(404))?;
        }
    }

    Ok(false)
}

fn content_type(value: &str) -> Header {
    Header::from_bytes("Content-Type", value).unwrap()
}

fn open_browser(port: u16) {
    let url = format!("http://localhost:{port}/");

    let safari = std::process::Command::new("open")
        .args(["-a", "Safari", &url])
        .spawn();

    if safari.is_err() {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

fn extract_token_from_json(body: &str) -> Option<String> {
    let pos = body.find("\"token\"")?;
    let after = body[pos + 7..].trim_start().strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    let t = &after[..end];
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn extract_token_from_url(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "ckWebAuthToken" {
            let token = percent_decode(value);
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let (Some(a), Some(b)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((a * 16 + b) as char);
                    i += 3;
                    continue;
                }
                out.push('%');
                i += 1;
            }
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }

    out
}

fn redact_auth_url(url: &str) -> String {
    redact_query_value(url, "ckWebAuthToken=")
}

fn redact_auth_body(body: &str) -> String {
    if let Some(token) = extract_token_from_json(body) {
        body.replace(&token, &redact_secret(&token))
    } else {
        body.to_string()
    }
}

fn redact_query_value(url: &str, key: &str) -> String {
    let Some(start) = url.find(key) else {
        return url.to_string();
    };
    let value_start = start + key.len();
    let value_end = url[value_start..]
        .find('&')
        .map(|offset| value_start + offset)
        .unwrap_or(url.len());
    let value = &url[value_start..value_end];
    let replacement = redact_secret(value);

    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..value_start]);
    out.push_str(&replacement);
    out.push_str(&url[value_end..]);
    out
}

fn redact_secret(value: &str) -> String {
    if value.len() <= 8 {
        "***".to_string()
    } else {
        format!("{}...{}", &value[..4], &value[value.len() - 4..])
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

fn success_html() -> &'static str {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>bear-cli — Authenticated</title>
    <style>
        body {
            margin: 0;
            min-height: 100vh;
            display: grid;
            place-items: center;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
            background: #f9fafb;
            color: #111827;
        }
        .card {
            width: min(420px, calc(100vw - 32px));
            padding: 28px;
            border: 1px solid #e5e7eb;
            border-radius: 16px;
            background: #fff;
            text-align: center;
        }
        h1 { margin: 0 0 8px; font-size: 22px; }
        p { margin: 0; color: #4b5563; line-height: 1.5; }
    </style>
</head>
<body>
    <div class="card">
        <h1>Authenticated</h1>
        <p>The CloudKit token was received. You can close this tab.</p>
    </div>
</body>
</html>"#
}

fn callback_html(port: u16) -> String {
    CALLBACK_HTML_TEMPLATE.replace("__PORT__", &port.to_string())
}

fn auth_html(port: u16) -> String {
    AUTH_HTML_TEMPLATE
        .replace("__PORT__", &port.to_string())
        .replace("__API_TOKEN__", API_TOKEN)
}

const AUTH_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>bear-cli — Sign In</title>
    <style>
        :root {
            --bg:#ffffff; --bg-card:#f9fafb; --text:#1a1a2e; --text-muted:#6b7280;
            --accent:#dd4c4f; --accent-hover:#c43c3f; --border:#e5e7eb;
            --input-bg:#ffffff; --input-border:#d1d5db; --code-bg:#e5e7eb;
        }
        @media (prefers-color-scheme: dark) {
            :root {
                --bg:#0f1117; --bg-card:#1a1b23; --text:#e2e8f0; --text-muted:#94a3b8;
                --accent:#dd4c4f; --accent-hover:#e86568; --border:#2d2d3d;
                --input-bg:#1e1f2a; --input-border:#3d3e4d; --code-bg:#2d2d3d;
            }
        }
        * { margin:0; padding:0; box-sizing:border-box; }
        body {
            font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;
            background:var(--bg); min-height:100vh;
            display:flex; align-items:center; justify-content:center;
            color:var(--text); -webkit-font-smoothing:antialiased;
        }
        .container {
            background:var(--bg-card); border:1px solid var(--border);
            border-radius:16px; padding:36px; max-width:380px; width:90%; text-align:center;
        }
        h1 { font-size:22px; font-weight:700; margin-bottom:6px; letter-spacing:-0.3px; }
        .subtitle { font-size:14px; color:var(--text-muted); margin-bottom:32px; line-height:1.5; }
        #apple-sign-in-button { position:absolute; width:1px; height:1px; overflow:hidden; opacity:0; pointer-events:none; }
        .custom-apple-btn {
            display:none; align-items:center; justify-content:center; gap:8px;
            padding:0 28px; height:46px; background:var(--accent); color:#fff;
            border:none; border-radius:10px; font-size:15px; font-weight:600;
            cursor:pointer; transition:background 0.2s;
        }
        .custom-apple-btn:hover { background:var(--accent-hover); }
        #apple-sign-out-button { display:none; }
        .status { margin-top:20px; font-size:13px; color:var(--text-muted); min-height:20px; }
        .status.error { color:#ef4444; }
        .status.success { color:#22c55e; font-weight:600; }
        .manual {
            display:none; margin-top:28px; padding:20px;
            background:#f3f4f6; border:1px solid var(--border);
            border-radius:10px; text-align:left; font-size:13px; line-height:1.7; color:var(--text-muted);
        }
        @media (prefers-color-scheme:dark) { .manual { background:#161722; } }
        .manual strong { color:var(--text); }
        .manual ol { margin:10px 0 12px 18px; }
        .manual a { color:var(--accent); text-decoration:none; }
        .manual code {
            background:var(--code-bg); padding:2px 6px; border-radius:4px;
            font-size:11px; color:var(--text);
            font-family:'SF Mono',SFMono-Regular,Menlo,Consolas,monospace;
        }
        .manual input {
            width:100%; font-size:12px; background:var(--input-bg); color:var(--text);
            border:1px solid var(--input-border); border-radius:8px;
            padding:10px 12px; margin-top:10px; outline:none; transition:border-color 0.2s;
            font-family:'SF Mono',SFMono-Regular,Menlo,Consolas,monospace;
        }
        .manual input:focus { border-color:var(--accent); }
        .manual button {
            margin-top:10px; padding:9px 20px; background:var(--accent); color:#fff;
            border:none; border-radius:8px; font-size:13px; font-weight:600;
            cursor:pointer; transition:background 0.2s;
        }
        .manual button:hover { background:var(--accent-hover); }
    </style>
</head>
<body>
    <div class="container">
        <h1>bear-cli</h1>
        <p class="subtitle">Sign in with your Apple ID to connect to your Bear notes via iCloud.</p>
        <button class="custom-apple-btn" id="custom-apple-btn"
            onclick="document.querySelector('#apple-sign-in-button .apple-auth-button').click()">
            <svg width="16" height="19" viewBox="0 0 16 19" fill="none">
                <path d="M13.2 9.94c-.02-2.08 1.7-3.08 1.78-3.13-1-1.4-2.5-1.6-3.02-1.62-1.27-.13-2.52.76-3.17.76-.67 0-1.68-.74-2.77-.72A4.08 4.08 0 002.57 7.4c-1.5 2.58-.38 6.38 1.05 8.47.72 1.02 1.56 2.17 2.67 2.13 1.08-.04 1.49-.69 2.79-.69 1.29 0 1.66.69 2.78.66 1.16-.02 1.88-1.03 2.58-2.06.83-1.18 1.16-2.34 1.18-2.4-.03-.01-2.24-.85-2.26-3.4l-.14.83zM10.93 3.52A3.75 3.75 0 0011.8.5a3.86 3.86 0 00-2.5 1.3 3.6 3.6 0 00-.9 2.9 3.2 3.2 0 002.53-1.18z" fill="#fff"/>
            </svg>
            Sign in with Apple
        </button>
        <div id="apple-sign-in-button"></div>
        <div id="apple-sign-out-button"></div>
        <script>
            (function() {
                var btn = document.getElementById('apple-sign-in-button');
                var custom = document.getElementById('custom-apple-btn');
                var obs = new MutationObserver(function() {
                    if (btn.querySelector('.apple-auth-button')) {
                        obs.disconnect();
                        custom.style.display = 'inline-flex';
                    }
                });
                obs.observe(btn, {childList:true, subtree:true});
            })();
        </script>
        <p class="status" id="status">Loading CloudKit JS...</p>
        <div class="manual" id="manual">
            <strong>Manual token entry</strong>
            <ol>
                <li>Open <a href="https://web.bear.app" target="_blank">web.bear.app</a> and sign in</li>
                <li>Open DevTools (Cmd+Option+I) &rarr; Network tab</li>
                <li>Look for requests to <code>apple-cloudkit.com</code></li>
                <li>Copy the <code>ckWebAuthToken</code> value from any request URL</li>
            </ol>
            <input type="text" id="manual-token" placeholder="Paste ckWebAuthToken here">
            <button onclick="submitManualToken()">Submit Token</button>
        </div>
    </div>
    <script>
        const PORT = __PORT__;

        function setStatus(msg, cls) {
            const el = document.getElementById('status');
            el.textContent = msg;
            el.className = 'status' + (cls ? ' ' + cls : '');
        }

        function showManual() {
            document.getElementById('manual').style.display = 'block';
        }

        let capturedToken = null;

        function checkURLForToken(url) {
            if (typeof url === 'string' && url.includes('ckWebAuthToken=')) {
                const m = url.match(/ckWebAuthToken=([^&]+)/);
                if (m && m[1] !== 'null' && m[1] !== 'undefined') {
                    capturedToken = decodeURIComponent(m[1]);
                    return true;
                }
            }
            return false;
        }

        if (checkURLForToken(window.location.href)) {
            setStatus('Received token from redirect URL. Finalizing sign-in...');
            if (window.history && window.history.replaceState) {
                window.history.replaceState({}, document.title, '/');
            }
            setTimeout(function() {
                sendToken(capturedToken).then(function(sent) {
                    if (!sent) {
                        setStatus('Could not reach CLI. Use the manual flow below.', 'error');
                        showManual();
                        document.getElementById('manual-token').value = capturedToken;
                    }
                });
            }, 0);
        }

        function looksLikeToken(value) {
            return typeof value === 'string' && value.length > 20 && value !== 'null' && value !== 'undefined';
        }

        function scanValueForToken(value, seen) {
            if (looksLikeToken(value)) {
                return value;
            }
            if (!value || typeof value !== 'object') {
                return null;
            }
            if (!seen) {
                seen = new Set();
            }
            if (seen.has(value)) {
                return null;
            }
            seen.add(value);

            if (Array.isArray(value)) {
                for (const item of value) {
                    const found = scanValueForToken(item, seen);
                    if (found) return found;
                }
                return null;
            }

            const keys = Object.keys(value);
            const preferred = keys.filter(function(key) {
                const lower = key.toLowerCase();
                return lower.includes('token') || lower.includes('auth') || lower.includes('session');
            });
            const ordered = preferred.concat(keys.filter(function(key) { return !preferred.includes(key); }));

            for (const key of ordered) {
                const found = scanValueForToken(value[key], seen);
                if (found) return found;
            }
            return null;
        }

        function captureToken(candidate) {
            if (looksLikeToken(candidate)) {
                capturedToken = candidate;
                return true;
            }
            const found = scanValueForToken(candidate);
            if (found) {
                capturedToken = found;
                return true;
            }
            return false;
        }

        function inspectStorage() {
            const stores = [window.localStorage, window.sessionStorage];
            for (const store of stores) {
                if (!store) continue;
                for (let i = 0; i < store.length; i++) {
                    const key = store.key(i);
                    if (!key) continue;
                    let raw = null;
                    try { raw = store.getItem(key); } catch (e) {}
                    if (!raw) continue;
                    if (captureToken(raw)) return true;
                    try {
                        if (captureToken(JSON.parse(raw))) return true;
                    } catch (e) {}
                }
            }
            return false;
        }

        function inspectContainerSession(container) {
            try {
                const sessions = container && container._sessions;
                if (!sessions) return false;
                if (captureToken(sessions.production)) return true;
                return captureToken(sessions);
            } catch (e) {
                return false;
            }
        }

        const origOpen = XMLHttpRequest.prototype.open;
        XMLHttpRequest.prototype.open = function(method, url) {
            checkURLForToken(url);
            return origOpen.apply(this, arguments);
        };

        const origFetch = window.fetch;
        window.fetch = function(input, init) {
            checkURLForToken(typeof input === 'string' ? input : (input && input.url) || '');
            return origFetch.apply(this, arguments);
        };

        window.addEventListener('message', function(event) {
            try {
                const d = (typeof event.data === 'string') ? JSON.parse(event.data) : event.data;
                const t = d && (d.ckWebAuthToken || d.webAuthToken || d.authToken || d.ckSession);
                if (!captureToken(t)) {
                    captureToken(d);
                    const callbackUrl = d && (d.callbackUrl || d.url);
                    if (callbackUrl) checkURLForToken(callbackUrl);
                }
            } catch(e) {}
        });

        async function sendToken(token) {
            try {
                const r = await fetch('http://localhost:' + PORT + '/callback', {
                    method: 'POST',
                    headers: {'Content-Type': 'application/json'},
                    body: JSON.stringify({token: token})
                });
                if (r.ok) {
                    document.getElementById('custom-apple-btn').style.display = 'none';
                    document.getElementById('apple-sign-in-button').style.display = 'none';
                    document.getElementById('manual').style.display = 'none';
                    var n = 5;
                    setStatus('Authenticated! Closing in ' + n + 's...', 'success');
                    var t = setInterval(function() {
                        n--;
                        if (n <= 0) {
                            clearInterval(t);
                            window.close();
                            setTimeout(function() {
                                setStatus('Authenticated! You can close this tab.', 'success');
                            }, 500);
                        } else {
                            setStatus('Authenticated! Closing in ' + n + 's...', 'success');
                        }
                    }, 1000);
                    return true;
                }
            } catch(e) {}
            return false;
        }

        function submitManualToken() {
            const t = document.getElementById('manual-token').value.trim();
            if (t) sendToken(t);
            else setStatus('Paste a token first', 'error');
        }

        async function onSignedIn(container) {
            setStatus('Signed in. Retrieving token...');
            if (inspectContainerSession(container) || inspectStorage()) {
                const sent = await sendToken(capturedToken);
                if (sent) return;
            }
            if (!capturedToken) {
                try {
                    const db = container.getDatabaseWithDatabaseScope(CloudKit.DatabaseScope.PRIVATE);
                    await db.performQuery({recordType:'SFNoteTag'}, {zoneName:'Notes'}, {resultsLimit:1}).catch(function() {});
                } catch(e) {}
            }
            if (!capturedToken) {
                inspectContainerSession(container);
                inspectStorage();
            }
            if (capturedToken) {
                const sent = await sendToken(capturedToken);
                if (!sent) {
                    setStatus('Could not reach CLI. Use the manual flow below.', 'error');
                    showManual();
                    document.getElementById('manual-token').value = capturedToken;
                }
            } else {
                setStatus('Could not extract token automatically.', 'error');
                showManual();
            }
        }
    </script>
    <script src="https://cdn.apple-cloudkit.com/ck/2/cloudkit.js"
        onerror="setStatus('CloudKit JS failed to load.', 'error'); showManual();"></script>
    <script>
        (function() {
            if (typeof CloudKit === 'undefined') {
                setStatus('CloudKit JS not available.', 'error');
                showManual();
                return;
            }
            CloudKit.configure({
                containers: [{
                    containerIdentifier: 'iCloud.net.shinyfrog.bear',
                    apiTokenAuth: {
                        apiToken: '__API_TOKEN__',
                        persist: true,
                        signInButton: { id: 'apple-sign-in-button', theme: 'white' },
                        signOutButton: { id: 'apple-sign-out-button' }
                    },
                    environment: 'production'
                }]
            });
            var container = CloudKit.getDefaultContainer();
            container.setUpAuth().then(function(uid) {
                setStatus('');
                if (uid) onSignedIn(container);
            }).catch(function() {
                setStatus('CloudKit auth setup failed. Use manual flow.', 'error');
                showManual();
            });
            container.whenUserSignsIn().then(function() {
                onSignedIn(container);
            }).catch(function() {});
        })();
    </script>
</body>
</html>"##;

const CALLBACK_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>bear-cli — Completing Sign-In</title>
    <style>
        body {
            margin: 0;
            min-height: 100vh;
            display: grid;
            place-items: center;
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
            background: #f9fafb;
            color: #111827;
        }
        .card {
            width: min(420px, calc(100vw - 32px));
            padding: 28px;
            border: 1px solid #e5e7eb;
            border-radius: 16px;
            background: #fff;
            text-align: center;
        }
        h1 { margin: 0 0 8px; font-size: 22px; }
        p { margin: 0; color: #4b5563; line-height: 1.5; }
    </style>
</head>
<body>
    <div class="card">
        <h1>Completing Sign-In</h1>
        <p id="status">Passing the CloudKit session back to bear-cli...</p>
    </div>
    <script>
        const PORT = __PORT__;

        function extractToken(url) {
            const match = String(url || '').match(/[?&]ckWebAuthToken=([^&]+)/);
            return match ? decodeURIComponent(match[1]) : null;
        }

        async function finish() {
            const href = window.location.href;
            const token = extractToken(href);

            if (window.opener && !window.opener.closed) {
                try {
                    window.opener.postMessage({ callbackUrl: href }, '*');
                } catch (e) {}
            }

            if (token) {
                try {
                    const r = await fetch('http://localhost:' + PORT + '/callback', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ token: token })
                    });
                    if (r.ok) {
                        document.getElementById('status').textContent = 'Authenticated. You can close this tab.';
                        window.close();
                        return;
                    }
                } catch (e) {}
            }

            document.getElementById('status').textContent = 'Waiting for the main window to finish sign-in...';
            setTimeout(function() { window.close(); }, 1200);
        }

        finish();
    </script>
</body>
</html>"##;
