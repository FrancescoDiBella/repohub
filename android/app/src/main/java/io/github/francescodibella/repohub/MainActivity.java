package io.github.francescodibella.repohub;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;
import android.os.Bundle;
import android.os.Message;
import android.webkit.JavascriptInterface;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * The app's one screen: RepoHub's interface (Rust compiled to WebAssembly, drawn with WebGPU)
 * in a WebView. The page is served from the APK under {@link #BASE}; its requests come through
 * {@link Bridge} to {@link Backend}, which holds the account and the drawings on the device.
 */
public class MainActivity extends Activity {
    /** A host reserved for apps' own files: nothing on the internet answers to it. */
    static final String HOST = "appassets.androidplatform.net";
    static final String BASE = "https://" + HOST + "/";

    private final ExecutorService pool = Executors.newFixedThreadPool(4);
    private WebView web;
    private Backend backend;
    /** True while the page on screen is the app's own: only then does the bridge answer. */
    private volatile boolean ownPage;
    private static final int SAVE_FILE = 41;
    private static final int OPEN_FILE = 42;
    /** The board waiting for the "save to" picker (Export), and who asked. */
    private String saveContent;
    private int saveId;
    private int openId;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        backend = new Backend(new AccountStore(this), new GitHubHttp(), getFilesDir());
        web = new WebView(this);
        web.setBackgroundColor(0xff0d1117);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(false);
        s.setAllowContentAccess(false);
        s.setSupportMultipleWindows(true);
        s.setTextZoom(100);
        s.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        web.setOverScrollMode(WebView.OVER_SCROLL_NEVER);
        web.addJavascriptInterface(new Bridge(), "RepoHubNative");
        web.setWebViewClient(new Client());
        web.setWebChromeClient(new Chrome());
        setContentView(web);
        if (saved == null || web.restoreState(saved) == null) {
            web.loadUrl(BASE + "index.html");
        }
    }

    @Override
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        web.saveState(out);
    }

    @Override
    protected void onDestroy() {
        pool.shutdownNow();
        web.destroy();
        super.onDestroy();
    }

    /** Back goes back inside the app first; the page calls {@link Bridge#exit} when it has nowhere to go. */
    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (ownPage) {
            web.evaluateJavascript("globalThis.__repohubBack ? (globalThis.__repohubBack(), true) : false", handled -> {
                if (!"true".equals(handled)) {
                    finish();
                }
            });
        } else {
            finish();
        }
    }

    /** Answers a picker the page asked for (see `src/board_file.rs`). */
    private void fileReply(int id, Object reply) {
        final String js = "globalThis.__repohubFile && globalThis.__repohubFile(" + id + "," + (reply instanceof String ? JSONObject.quote((String) reply) : String.valueOf(reply)) + ")";
        runOnUiThread(() -> {
            if (ownPage) {
                web.evaluateJavascript(js, null);
            }
        });
    }

    @Override
    @SuppressWarnings("deprecation")
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        final Uri uri = data == null ? null : data.getData();
        if (request == SAVE_FILE) {
            final int id = saveId;
            final byte[] bytes = saveContent == null ? new byte[0] : saveContent.getBytes(StandardCharsets.UTF_8);
            saveContent = null;
            if (result != RESULT_OK || uri == null) {
                fileReply(id, "cancelled");
                return;
            }
            pool.execute(() -> {
                try (OutputStream out = getContentResolver().openOutputStream(uri, "wt")) {
                    if (out == null) {
                        throw new IOException("the file can't be written");
                    }
                    out.write(bytes);
                    fileReply(id, "saved");
                } catch (Exception e) {
                    fileReply(id, "error: " + e.getMessage());
                }
            });
        } else if (request == OPEN_FILE) {
            final int id = openId;
            if (result != RESULT_OK || uri == null) {
                fileReply(id, JSONObject.NULL);
                return;
            }
            pool.execute(() -> {
                String name = "drawing.excalidraw";
                try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
                    if (c != null && c.moveToFirst() && !c.isNull(0)) {
                        name = c.getString(0);
                    }
                } catch (Exception e) {
                    // The name is only a suggestion.
                }
                try (InputStream in = getContentResolver().openInputStream(uri)) {
                    if (in == null) {
                        throw new IOException("the file can't be read");
                    }
                    ByteArrayOutputStream buf = new ByteArrayOutputStream();
                    byte[] chunk = new byte[16 * 1024];
                    int n;
                    while ((n = in.read(chunk)) > 0) {
                        buf.write(chunk, 0, n);
                        if (buf.size() > Notes.MAX_BYTES) {
                            fileReply(id, "error: the file is larger than 25 MB");
                            return;
                        }
                    }
                    fileReply(id, new JSONArray().put(name).put(new String(buf.toByteArray(), StandardCharsets.UTF_8)));
                } catch (Exception e) {
                    fileReply(id, "error: " + e.getMessage());
                }
            });
        }
    }

    private void openOutside(Uri uri) {
        String scheme = uri.getScheme();
        if (!"https".equals(scheme) && !"http".equals(scheme) && !"mailto".equals(scheme)) {
            return;
        }
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE));
        } catch (ActivityNotFoundException e) {
            // No browser on this device: nothing to open it with.
        }
    }

    static String mime(String path) {
        String ext = path.substring(path.lastIndexOf('.') + 1);
        switch (ext) {
            case "html": return "text/html";
            case "js": return "text/javascript";
            case "wasm": return "application/wasm";
            case "json": return "application/json";
            case "css": return "text/css";
            case "png": return "image/png";
            case "svg": return "image/svg+xml";
            case "webmanifest": return "application/manifest+json";
            default: return "application/octet-stream";
        }
    }

    private final class Client extends WebViewClient {
        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            Uri uri = request.getUrl();
            if (!HOST.equals(uri.getHost())) {
                return null; // pictures in a README, avatars: fetched as any page would
            }
            String path = uri.getPath() == null ? "" : uri.getPath();
            while (path.startsWith("/")) {
                path = path.substring(1);
            }
            if (path.isEmpty()) {
                path = "index.html";
            }
            Map<String, String> headers = new HashMap<>();
            headers.put("Cache-Control", "no-store");
            if (path.contains("..") || path.contains("\\") || !"GET".equals(request.getMethod())) {
                return new WebResourceResponse("text/plain", "utf-8", 404, "Not Found", headers, new ByteArrayInputStream(new byte[0]));
            }
            try {
                InputStream in = getAssets().open("web/" + path);
                String type = mime(path);
                return new WebResourceResponse(type, type.startsWith("text/") || type.endsWith("json") ? "utf-8" : null, 200, "OK", headers, in);
            } catch (IOException e) {
                return new WebResourceResponse("text/plain", "utf-8", 404, "Not Found", headers, new ByteArrayInputStream(new byte[0]));
            }
        }

        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            if (HOST.equals(request.getUrl().getHost())) {
                return false;
            }
            // Anything else is a link: it opens in the browser, never inside the app.
            openOutside(request.getUrl());
            return true;
        }

        @Override
        public void onPageStarted(WebView view, String url, android.graphics.Bitmap icon) {
            ownPage = url != null && url.startsWith(BASE);
        }
    }

    private final class Chrome extends WebChromeClient {
        /** `window.open` (the app's links): the address goes to the browser. */
        @Override
        public boolean onCreateWindow(WebView view, boolean dialog, boolean userGesture, Message result) {
            if (!userGesture) {
                return false;
            }
            final WebView temp = new WebView(MainActivity.this);
            temp.setWebViewClient(new WebViewClient() {
                @Override
                public boolean shouldOverrideUrlLoading(WebView v, WebResourceRequest request) {
                    openOutside(request.getUrl());
                    v.destroy();
                    return true;
                }
            });
            ((WebView.WebViewTransport) result.obj).setWebView(temp);
            result.sendToTarget();
            return true;
        }
    }

    /** What the page can call. It never hands out the token. */
    private final class Bridge {
        @JavascriptInterface
        public void request(final int id, final String method, final String url, final String accept, final String body, final String ifNoneMatch) {
            if (!ownPage) {
                return;
            }
            pool.execute(() -> {
                Backend.Reply r;
                try {
                    r = backend.handle(method, url, accept, body, ifNoneMatch == null || ifNoneMatch.isEmpty() ? null : ifNoneMatch);
                } catch (Throwable t) {
                    r = Backend.Reply.text(500, String.valueOf(t.getMessage()));
                }
                JSONArray a = new JSONArray();
                a.put(r.status).put(r.body).put(r.remaining).put(r.reset).put(r.retryAfter).put(r.link).put(r.etag);
                final String js = "globalThis.__repohubNative(" + id + "," + a + ")";
                runOnUiThread(() -> {
                    if (ownPage) {
                        web.evaluateJavascript(js, null);
                    }
                });
            });
        }

        /** Export: the system's "save to" picker for `name`, then `content` is written there. */
        @JavascriptInterface
        @SuppressWarnings("deprecation")
        public void saveFile(final int id, final String name, final String content) {
            if (!ownPage) {
                return;
            }
            runOnUiThread(() -> {
                saveId = id;
                saveContent = content;
                Intent i = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("application/json").putExtra(Intent.EXTRA_TITLE, name);
                try {
                    startActivityForResult(i, SAVE_FILE);
                } catch (ActivityNotFoundException e) {
                    saveContent = null;
                    fileReply(id, "error: this phone has no app to save files with");
                }
            });
        }

        /** Import: the system's file picker; the page gets the file's name and text. */
        @JavascriptInterface
        @SuppressWarnings("deprecation")
        public void openFile(final int id) {
            if (!ownPage) {
                return;
            }
            runOnUiThread(() -> {
                openId = id;
                // Any type: phones rarely know .excalidraw, and the page checks the content.
                Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*");
                try {
                    startActivityForResult(i, OPEN_FILE);
                } catch (ActivityNotFoundException e) {
                    fileReply(id, "error: this phone has no app to pick files with");
                }
            });
        }

        @JavascriptInterface
        public void exit() {
            runOnUiThread(MainActivity.this::finish);
        }
    }
}
