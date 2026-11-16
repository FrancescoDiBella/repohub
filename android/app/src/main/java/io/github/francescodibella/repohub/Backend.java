package io.github.francescodibella.repohub;

import org.json.JSONObject;

import java.io.File;

/**
 * Answers the page's requests on the device, as repohub-server does for the web build:
 * the configuration, the account, GitHub (with the saved token) and the drawings.
 * The token goes from {@link Account} to GitHub and nowhere else; no answer carries it.
 */
final class Backend {
    static final String API = "https://api.github.com";

    /** Where the account is kept. */
    interface Account {
        /** The saved user name, or null. */
        String user();

        /** The saved token, or null. */
        String token();

        /** Replaces the account; null removes a value. */
        void save(String user, String token) throws Exception;
    }

    /** Sends one request to GitHub. */
    interface Http {
        Reply send(String method, String url, String accept, String token, String body) throws Exception;
    }

    /** What the page gets back: the six values `gh_fetch` returns (src/net.rs). */
    static final class Reply {
        final int status;
        final String body;
        String remaining = "";
        String reset = "";
        String retryAfter = "";
        String link = "";

        Reply(int status, String body) {
            this.status = status;
            this.body = body;
        }

        static Reply text(int status, String body) {
            return new Reply(status, body);
        }

        static Reply json(int status, String body) {
            return new Reply(status, body);
        }
    }

    private final Account account;
    private final Http http;
    private final Notes notes;

    Backend(Account account, Http http, File dataDir) {
        this.account = account;
        this.http = http;
        this.notes = new Notes(dataDir);
    }

    static String jsonString(String s) {
        return JSONObject.quote(s);
    }

    private static String blankToNull(String s) {
        return s == null || s.trim().isEmpty() ? null : s.trim();
    }

    /** `config.json`: what the page may know. Whether a token is saved, never the token. */
    String config() {
        try {
            JSONObject o = new JSONObject();
            o.put("device", true);
            o.put("accountEditable", true);
            o.put("notes", true);
            o.put("allowWrites", true);
            o.put("api", API);
            o.put("hasToken", blankToNull(account.token()) != null);
            String user = blankToNull(account.user());
            if (user != null) {
                o.put("user", user);
            }
            return o.toString();
        } catch (Exception e) {
            return "{\"device\":true,\"accountEditable\":true,\"notes\":true}";
        }
    }

    private Reply saveAccount(String body) {
        try {
            JSONObject o = new JSONObject(body);
            if (o.optBoolean("signOut", false)) {
                account.save(null, null);
                return Reply.json(200, config());
            }
            String user = blankToNull(o.isNull("user") ? null : o.optString("user", ""));
            // No token in the request keeps the saved one.
            String token = o.isNull("token") ? blankToNull(account.token()) : blankToNull(o.optString("token", ""));
            if (user == null && token == null) {
                return Reply.text(400, "Type your GitHub user name, a token, or both.");
            }
            if (user != null && !user.matches("[A-Za-z0-9-]{1,39}")) {
                return Reply.text(400, "That isn't a GitHub user name: letters, digits and hyphens only.");
            }
            if (token != null && !token.matches("[\\x21-\\x7E]{1,255}")) {
                return Reply.text(400, "That isn't a GitHub token: it can't contain spaces or line breaks.");
            }
            account.save(user, token);
            return Reply.json(200, config());
        } catch (Exception e) {
            return Reply.text(500, "The account wasn't saved: " + e.getMessage());
        }
    }

    private Reply github(String method, String pathAndQuery, String accept, String body) {
        for (int i = 0; i < pathAndQuery.length(); i++) {
            if (Character.isISOControl(pathAndQuery.charAt(i))) {
                return Reply.text(403, "Invalid characters in URL");
            }
        }
        int q = pathAndQuery.indexOf('?');
        String rawPath = q < 0 ? pathAndQuery : pathAndQuery.substring(0, q);
        String decoded = GitHubPaths.decode(rawPath);
        if (decoded == null) {
            return Reply.text(403, "Path traversal or invalid characters in path");
        }
        if (!GitHubPaths.allowed(method, decoded)) {
            return Reply.text(403, "This request isn't one RepoHub makes");
        }
        if (body.length() > (1 << 16)) {
            return Reply.text(413, "Too much text to send");
        }
        try {
            String type = accept == null || accept.isEmpty() ? "application/vnd.github+json" : accept;
            return http.send(method, API + pathAndQuery, type, blankToNull(account.token()), body);
        } catch (Exception e) {
            return Reply.text(502, "Can't reach GitHub: " + e.getMessage());
        }
    }

    /** Answers one request of the page; `url` is relative to the page ("gh/user", "config.json"). */
    Reply handle(String method, String url, String accept, String body) {
        String u = url.startsWith("/") ? url.substring(1) : url;
        if (body == null) {
            body = "";
        }
        int q = u.indexOf('?');
        String path = q < 0 ? u : u.substring(0, q);
        if (u.startsWith("gh/")) {
            return github(method, u.substring(2), accept, body);
        }
        if (path.equals("api/notes") || path.equals("api/notes/file")) {
            return notes.handle(method, u, body);
        }
        if (path.equals("config.json")) {
            return method.equals("GET") ? Reply.json(200, config()) : Reply.text(405, "Method not allowed");
        }
        if (path.equals("api/account")) {
            return method.equals("PUT") ? saveAccount(body) : Reply.text(405, "Method not allowed");
        }
        return Reply.text(404, "Not found");
    }
}
