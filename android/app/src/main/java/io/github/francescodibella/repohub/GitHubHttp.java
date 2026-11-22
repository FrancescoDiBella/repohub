package io.github.francescodibella.repohub;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;

/** Requests to GitHub over HTTPS, with the phone's own certificate store. */
final class GitHubHttp implements Backend.Http {
    private static final int MAX_BYTES = 32 << 20;

    @Override
    public Backend.Reply send(String method, String url, String accept, String token, String body) throws Exception {
        String currentUrl = url;
        String currentToken = token;
        for (int hop = 0; ; hop++) {
            HttpURLConnection c = (HttpURLConnection) new URL(currentUrl).openConnection();
            try {
                c.setConnectTimeout(15_000);
                c.setReadTimeout(60_000);
                c.setInstanceFollowRedirects(false);
                c.setRequestMethod(method);
                c.setRequestProperty("Accept", hop == 0 ? accept : "*/*");
                if (hop == 0) {
                    c.setRequestProperty("X-GitHub-Api-Version", "2022-11-28");
                }
                c.setRequestProperty("User-Agent", "RepoHub-Android");
                if (currentToken != null) {
                    c.setRequestProperty("Authorization", "Bearer " + currentToken);
                }
                // Writes carry a JSON body; a body-less PUT (starring) still needs a length.
                if (hop == 0 && (!body.isEmpty() || method.equals("PUT"))) {
                    byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
                    c.setDoOutput(true);
                    c.setFixedLengthStreamingMode(bytes.length);
                    if (bytes.length > 0) {
                        c.setRequestProperty("Content-Type", "application/json");
                    }
                    try (OutputStream out = c.getOutputStream()) {
                        out.write(bytes);
                    }
                }
                int status = c.getResponseCode();
                if (method.equals("GET") && hop < 3 && (status == 301 || status == 302 || status == 303 || status == 307 || status == 308)) {
                    String location = header(c, "Location");
                    if (location.startsWith("https://")) {
                        currentUrl = location;
                        currentToken = null;
                        continue;
                    }
                }
                InputStream in = status >= 400 ? c.getErrorStream() : c.getInputStream();
                ByteArrayOutputStream data = new ByteArrayOutputStream();
                if (in != null) {
                    try (InputStream s = in) {
                        byte[] buf = new byte[16 * 1024];
                        int n;
                        while ((n = s.read(buf)) > 0 && data.size() < MAX_BYTES) {
                            data.write(buf, 0, n);
                        }
                    }
                }
                Backend.Reply r = new Backend.Reply(status, new String(data.toByteArray(), StandardCharsets.UTF_8));
                r.remaining = header(c, "x-ratelimit-remaining");
                r.reset = header(c, "x-ratelimit-reset");
                r.retryAfter = header(c, "retry-after");
                r.link = header(c, "link");
                return r;
            } finally {
                c.disconnect();
            }
        }
    }

    private static String header(HttpURLConnection c, String name) {
        String v = c.getHeaderField(name);
        return v == null ? "" : v;
    }
}
