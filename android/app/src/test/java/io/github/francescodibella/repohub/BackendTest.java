package io.github.francescodibella.repohub;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;

public class BackendTest {
    /** An account in memory. */
    static final class FakeAccount implements Backend.Account {
        String user;
        String token;

        @Override public String user() { return user; }
        @Override public String token() { return token; }
        @Override public void save(String user, String token) {
            this.user = user;
            this.token = token;
        }
    }

    /** Records what would go to GitHub. */
    static final class FakeHttp implements Backend.Http {
        final List<String> sent = new ArrayList<>();

        @Override
        public Backend.Reply send(String method, String url, String accept, String token, String body) {
            sent.add(method + " " + url + " token=" + token + " body=" + body);
            Backend.Reply r = new Backend.Reply(200, "{\"ok\":true}");
            r.remaining = "4999";
            return r;
        }
    }

    private File dir;
    private FakeAccount account;
    private FakeHttp http;
    private Backend backend;

    @Before
    public void setUp() throws Exception {
        dir = Files.createTempDirectory("repohub-android").toFile();
        account = new FakeAccount();
        http = new FakeHttp();
        backend = new Backend(account, http, dir);
    }

    @After
    public void tearDown() {
        delete(dir);
    }

    private static void delete(File f) {
        File[] kids = f.listFiles();
        if (kids != null) {
            for (File k : kids) {
                delete(k);
            }
        }
        f.delete();
    }

    @Test
    public void allowlistMatchesTheServers() {
        assertTrue(GitHubPaths.allowed("GET", "/notifications"));
        assertFalse(GitHubPaths.allowed("GET", "/notifications/threads/12"));
        assertTrue(GitHubPaths.allowed("PATCH", "/notifications/threads/12"));
        assertFalse(GitHubPaths.allowed("PATCH", "/notifications/threads/abc"));
        assertFalse(GitHubPaths.allowed("PUT", "/notifications"));
        assertFalse(GitHubPaths.allowed("DELETE", "/notifications/threads/12"));
        for (String p : new String[] {"/user", "/user/repos", "/user/starred", "/user/orgs", "/search/issues", "/users/octocat",
            "/users/octocat/repos", "/users/octocat/orgs", "/orgs/acme/repos", "/repos/o/r/actions/runs", "/repos/o/r/stats/participation",
            "/repos/o/r/contents", "/repos/o/r/readme", "/repos/o/r/languages", "/repos/o/r/commits", "/repos/o/r/branches", "/repos/o/r/issues",
            "/repos/o/r/git/ref/heads/feature/x", "/repos/o/r/pulls/12/files", "/repos/o/r/issues/7", "/repos/o/r/issues/7/comments",
            "/repos/o/r/pulls/12", "/repos/o/r/pulls/12/reviews"}) {
            assertTrue(p, GitHubPaths.allowed("GET", p));
        }
        for (String p : new String[] {"/user/emails", "/user/keys", "/repos/o/r", "/repos/o/r/hooks", "/repos/o/r/pulls/x/files", "/gists",
            "/repos/o/r/actions/secrets", "/orgs/acme/members", "/repos/o/r/git/ref/heads/", "/repos/o/r/issues/x", "/repos/o/r/issues/7/events",
            "/repos/o/r/pulls/12/commits", "/repos/o/r/pulls/12/reviews/3"}) {
            assertFalse(p, GitHubPaths.allowed("GET", p));
        }
        assertTrue(GitHubPaths.allowed("PUT", "/repos/o/r/pulls/3/merge"));
        assertTrue(GitHubPaths.allowed("PUT", "/user/starred/o/r"));
        assertTrue(GitHubPaths.allowed("DELETE", "/user/starred/o/r"));
        assertFalse(GitHubPaths.allowed("DELETE", "/repos/o/r"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/issues"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/pulls"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/git/refs"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/issues/7/comments"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/pulls/12/reviews"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/pulls/12/merge"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/issues/x/comments"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/pulls/12/reviews/3/dismissals"));
        assertFalse(GitHubPaths.allowed("DELETE", "/repos/o/r/issues/comments/9"));
        assertFalse(GitHubPaths.allowed("POST", "/user/repos"));
        assertTrue(GitHubPaths.allowed("PATCH", "/repos/o/r/issues/7"));
        assertFalse(GitHubPaths.allowed("PATCH", "/repos/o/r"));
        assertFalse(GitHubPaths.allowed("TRACE", "/user"));
    }

    @Test
    public void pathsThatCouldEscapeAreRefused() {
        assertEquals("/user/repos", GitHubPaths.decode("/user/repos"));
        assertEquals("/repos/o/r/git/ref/heads/feature+x", GitHubPaths.decode("/repos/o/r/git/ref/heads/feature+x"));
        for (String p : new String[] {"/repos/o/r/%2e%2e/%2e%2e/user/emails", "/repos/o/r/%2E%2E/etc", "/repos/o/r/%2f/etc", "/repos/o/r/%5c/etc",
            "/repos/o/r/..", "/repos/o/r/...", "/repos/o/r/.", "/repos/o/r//contents", "/repos/o/r\\readme", "/repos/o/r\u0000readme",
            "/repos/o/r/%252e%252e", "/repos/o/r/%zz"}) {
            assertNull(p, GitHubPaths.decode(p));
        }
    }

    @Test
    public void githubRequestsCarryTheTokenAndOnlyAllowedOnesLeave() {
        account.token = "ghp_secret";
        Backend.Reply r = backend.handle("GET", "gh/user/repos?per_page=100", "application/vnd.github+json", "");
        assertEquals(200, r.status);
        assertEquals("4999", r.remaining);
        assertEquals("GET https://api.github.com/user/repos?per_page=100 token=ghp_secret body=", http.sent.get(0));
        assertEquals(403, backend.handle("GET", "gh/user/emails", "", "").status);
        assertEquals(403, backend.handle("GET", "gh/repos/o/r/%2e%2e/%2e%2e/user/emails", "", "").status);
        assertEquals(403, backend.handle("DELETE", "gh/repos/o/r", "", "").status);
        assertEquals("nothing else reached GitHub", 1, http.sent.size());
        // Without a token the request goes out without one (public data).
        account.token = null;
        backend.handle("POST", "gh/repos/o/r/issues", "", "{\"title\":\"x\"}");
        assertEquals("POST https://api.github.com/repos/o/r/issues token=null body={\"title\":\"x\"}", http.sent.get(1));
    }

    @Test
    public void theAccountIsSavedAndTheTokenNeverComesBack() throws Exception {
        JSONObject c = new JSONObject(backend.handle("GET", "config.json", "", "").body);
        assertTrue(c.getBoolean("device") && c.getBoolean("accountEditable") && c.getBoolean("notes"));
        assertFalse(c.getBoolean("hasToken"));
        assertFalse(c.has("user"));

        Backend.Reply r = backend.handle("PUT", "api/account", "", "{\"signOut\":false,\"user\":\"octocat\",\"token\":\"ghp_secret\"}");
        assertEquals(200, r.status);
        assertFalse("the answer has no token", r.body.contains("ghp_secret"));
        c = new JSONObject(r.body);
        assertEquals("octocat", c.getString("user"));
        assertTrue(c.getBoolean("hasToken"));
        assertEquals("ghp_secret", account.token);

        // No token in the request keeps the saved one.
        r = backend.handle("PUT", "api/account", "", "{\"signOut\":false,\"user\":\"hubot\",\"token\":null}");
        assertEquals(200, r.status);
        assertEquals("hubot", account.user);
        assertEquals("ghp_secret", account.token);

        assertEquals(400, backend.handle("PUT", "api/account", "", "{\"user\":\"not a name\",\"token\":null}").status);
        assertEquals(400, backend.handle("PUT", "api/account", "", "{\"user\":\"octocat\",\"token\":\"two words\"}").status);
        assertEquals("hubot", account.user);

        r = backend.handle("PUT", "api/account", "", "{\"signOut\":true}");
        assertEquals(200, r.status);
        assertNull(account.user);
        assertNull(account.token);
        assertEquals(400, backend.handle("PUT", "api/account", "", "{\"signOut\":false,\"user\":\"\",\"token\":null}").status);
        assertEquals(405, backend.handle("GET", "api/account", "", "").status);
        assertEquals(404, backend.handle("GET", "api/other", "", "").status);
    }

    @Test
    public void drawingsRoundTrip() throws Exception {
        String q = "?repo=Octo%2FRepo&branch=feature%2Fx";
        assertEquals("[]", backend.handle("GET", "api/notes" + q, "", "").body);
        assertEquals(404, backend.handle("GET", "api/notes/file" + q + "&name=Plan%20%28v2%29", "", "").status);
        assertEquals(204, backend.handle("PUT", "api/notes/file" + q + "&name=Plan%20%28v2%29", "", "{\"type\":\"excalidraw\"}").status);
        assertTrue(new File(dir, "notes/octo/repo/feature%2Fx/Plan%20%28v2%29.excalidraw").isFile());
        Backend.Reply got = backend.handle("GET", "api/notes/file" + q + "&name=Plan%20%28v2%29", "", "");
        assertEquals(200, got.status);
        assertEquals("{\"type\":\"excalidraw\"}", got.body);
        JSONArray list = new JSONArray(backend.handle("GET", "api/notes" + q, "", "").body);
        assertEquals(1, list.length());
        assertEquals("Plan (v2)", list.getJSONObject(0).getString("name"));
        assertEquals(21, list.getJSONObject(0).getInt("size"));
        assertNotNull(list.getJSONObject(0).get("updated"));
        // Another branch has its own drawings.
        assertEquals("[]", backend.handle("GET", "api/notes?repo=Octo%2FRepo&branch=main", "", "").body);
        assertEquals(204, backend.handle("DELETE", "api/notes/file" + q + "&name=Plan%20%28v2%29", "", "").status);
        assertEquals(404, backend.handle("DELETE", "api/notes/file" + q + "&name=Plan%20%28v2%29", "", "").status);
    }

    @Test
    public void drawingNamesCantLeaveTheirFolder() {
        String q = "?repo=o%2Fr&branch=main";
        assertEquals(400, backend.handle("PUT", "api/notes/file" + q + "&name=..%2Fx", "", "{}").status);
        assertEquals(400, backend.handle("PUT", "api/notes/file" + q + "&name=.hidden", "", "{}").status);
        assertEquals(400, backend.handle("PUT", "api/notes/file?repo=..%2Fetc&branch=main&name=x", "", "{}").status);
        assertEquals(400, backend.handle("PUT", "api/notes/file?repo=a%2Fb%2Fc&branch=main&name=x", "", "{}").status);
        assertEquals(400, backend.handle("PUT", "api/notes/file" + q + "&name=x", "", "not json").status);
        assertEquals("%2E.", Notes.segment(".."));
        // A branch named ".." stays one folder inside the repository's.
        assertEquals(204, backend.handle("PUT", "api/notes/file?repo=o%2Fr&branch=..&name=x", "", "{}").status);
        assertTrue(new File(dir, "notes/o/r/%2E./x.excalidraw").isFile());
    }
}
