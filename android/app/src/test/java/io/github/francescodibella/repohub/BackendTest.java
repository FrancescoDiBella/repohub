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
        public Backend.Reply send(String method, String url, String accept, String token, String body, String ifNoneMatch) {
            sent.add(method + " " + url + " token=" + token + " body=" + body + (ifNoneMatch == null ? "" : " if-none-match=" + ifNoneMatch));
            // Like GitHub: the same tag back means nothing changed.
            Backend.Reply r = "W/\"v1\"".equals(ifNoneMatch) ? new Backend.Reply(304, "") : new Backend.Reply(200, "{\"ok\":true}");
            r.remaining = "4999";
            r.etag = "W/\"v1\"";
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
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/commits/0123abcdef0123abcdef0123abcdef0123abcdef/check-runs"));
        assertFalse(GitHubPaths.allowed("GET", "/repos/o/r/commits/main/check-runs"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/commits/0123abc/check-runs"));
        // One commit by its id, and two branches compared, as the server allows them.
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/commits/0123abc"));
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/commits/0123abcdef0123abcdef0123abcdef0123abcdef"));
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/compare/main...feature"));
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/compare/main...feature/login?per_page=100"));
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/compare/release/1.2...fix/x-y_z"));
        for (String p : new String[] {"/repos/o/r/commits/0123ab", "/repos/o/r/commits/main", "/repos/o/r/commits/0123abc/comments",
            "/repos/o/r/commits/0123abc/pulls", "/repos/o/r/compare", "/repos/o/r/compare/main", "/repos/o/r/compare/main..feature",
            "/repos/o/r/compare/main....feature", "/repos/o/r/compare/...feature", "/repos/o/r/compare/main...",
            "/repos/o/r/compare/-main...feature", "/repos/o/r/compare/main...a..b", "/repos/o/r/compare/main...fea ture",
            "/repos/o/r/compare/main...HEAD~1", "/repos/o/r/compare/main...x:y"}) {
            assertFalse(p, GitHubPaths.allowed("GET", p));
        }
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/compare/main...feature"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/commits/0123abc"));
        assertNull(GitHubPaths.decode("/repos/o/r/compare/main%2e%2e%2efeature"));
        assertNull(GitHubPaths.decode("/repos/o/r/compare/main...feature%2fx"));
        assertTrue(GitHubPaths.allowed("PATCH", "/notifications/threads/12"));
        assertFalse(GitHubPaths.allowed("PATCH", "/notifications/threads/abc"));
        assertFalse(GitHubPaths.allowed("PUT", "/notifications"));
        assertFalse(GitHubPaths.allowed("DELETE", "/notifications/threads/12"));
        for (String p : new String[] {"/user", "/user/repos", "/user/starred", "/user/orgs", "/search/issues", "/users/octocat",
            "/users/octocat/repos", "/users/octocat/orgs", "/orgs/acme/repos", "/repos/o/r/actions/runs", "/repos/o/r/stats/participation",
            "/repos/o/r/contents", "/repos/o/r/readme", "/repos/o/r/languages", "/repos/o/r/commits", "/repos/o/r/branches", "/repos/o/r/issues",
            "/repos/o/r/git/ref/heads/feature/x", "/repos/o/r/pulls/12/files", "/repos/o/r/issues/7", "/repos/o/r/issues/7/comments",
            "/repos/o/r/pulls/12", "/repos/o/r/pulls/12/reviews", "/repos/o/r/actions/runs/42/jobs", "/repos/o/r/actions/jobs/99/logs"}) {
            assertTrue(p, GitHubPaths.allowed("GET", p));
        }
        for (String p : new String[] {"/user/emails", "/user/keys", "/repos/o/r/hooks", "/repos/o/r/pulls/x/files", "/gists",
            "/repos/o/r/actions/secrets", "/orgs/acme/members", "/repos/o/r/git/ref/heads/", "/repos/o/r/issues/x", "/repos/o/r/issues/7/events",
            "/repos/o/r/pulls/12/commits", "/repos/o/r/pulls/12/reviews/3", "/repos/o/r/actions/runs/x/jobs", "/repos/o/r/actions/jobs/x/logs",
            "/repos/o/r/actions/runs/42/logs"}) {
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
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/r/pulls/12/comments"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/pulls/12/comments"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/actions/runs/42/rerun"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/actions/runs/42/rerun-failed-jobs"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/actions/runs/x/rerun"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/actions/runs/42/cancel"));
        assertFalse(GitHubPaths.allowed("DELETE", "/repos/o/r/actions/runs/42/logs"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/pulls/12/comments/3/replies"));
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
    public void triagePathsMatchTheServers() {
        for (String p : new String[] {"/repos/o/r/labels", "/repos/o/r/labels?per_page=100", "/repos/o/r/assignees?per_page=100",
            "/repos/o/r/milestones?state=open&per_page=100"}) {
            assertTrue(p, GitHubPaths.allowed("GET", p));
        }
        for (String p : new String[] {"/repos/o/r/labels/bug", "/repos/o/r/assignees/alice", "/repos/o/r/milestones/3", "/repos/o/r/collaborators",
            "/repos/o/r/issues/7/labels", "/labels"}) {
            assertFalse(p, GitHubPaths.allowed("GET", p));
        }
        // "/repos/o/labels" is the repository named "labels" (read for board sync), not a list.
        assertTrue(GitHubPaths.allowed("GET", "/repos/o/labels"));
        assertFalse(GitHubPaths.allowed("GET", "/repos/o/labels/labels/bug"));
        for (String m : new String[] {"POST", "PUT", "PATCH", "DELETE"}) {
            assertFalse(m, GitHubPaths.allowed(m, "/repos/o/r/labels"));
            assertFalse(m, GitHubPaths.allowed(m, "/repos/o/r/milestones"));
        }
        assertTrue(GitHubPaths.allowed("PATCH", "/repos/o/r/issues/7"));
        assertFalse(GitHubPaths.allowed("PATCH", "/repos/o/r/pulls/7"));
        assertFalse(GitHubPaths.allowed("POST", "/repos/o/r/issues/7/labels"));
        assertFalse(GitHubPaths.allowed("DELETE", "/repos/o/r/issues/7/labels/bug"));
        assertTrue(GitHubPaths.allowed("POST", "/repos/o/r/pulls/7/requested_reviewers"));
        assertTrue(GitHubPaths.allowed("DELETE", "/repos/o/r/pulls/7/requested_reviewers"));
        for (String p : new String[] {"/repos/o/r/pulls/x/requested_reviewers", "/repos/o/r/pulls/7a/requested_reviewers", "/repos/o/r/pulls//requested_reviewers",
            "/repos/o/r/pulls/7/requested_reviewers/alice", "/repos/o/r/pulls/7/requested_reviewer", "/repos/o/r/issues/7/requested_reviewers",
            "/repos/o/r/requested_reviewers", "/repos/o/r/pulls/requested_reviewers"}) {
            assertFalse(p, GitHubPaths.allowed("POST", p));
            assertFalse(p, GitHubPaths.allowed("DELETE", p));
        }
        assertFalse(GitHubPaths.allowed("PUT", "/repos/o/r/pulls/7/requested_reviewers"));
        assertFalse(GitHubPaths.allowed("PATCH", "/repos/o/r/pulls/7/requested_reviewers"));
        // DELETE anywhere else in a repository stays refused.
        for (String p : new String[] {"/repos/o/r", "/repos/o/r/pulls/7", "/repos/o/r/issues/7", "/repos/o/r/pulls/7/reviews/1", "/repos/o/r/pulls/7/merge",
            "/repos/o/r/issues/7/comments", "/repos/o/r/labels", "/repos/o/r/git/refs/heads/main"}) {
            assertFalse(p, GitHubPaths.allowed("DELETE", p));
        }
        assertNull(GitHubPaths.decode("/repos/o/r/pulls/7%2frequested_reviewers"));
    }

    @Test
    public void boardSyncPathsAreAsNarrowAsTheServers() {
        String board = "/repos/me/repohub-notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw";
        for (String m : new String[] {"GET", "PUT", "DELETE"}) {
            assertTrue(m, GitHubPaths.allowed(m, board));
            assertTrue(m, GitHubPaths.allowed(m, "/repos/me/notes/contents/boards/o/r/main/Plan.excalidraw"));
        }
        assertFalse(GitHubPaths.allowed("POST", board));
        assertFalse(GitHubPaths.allowed("PATCH", board));
        assertTrue(GitHubPaths.allowed("GET", "/repos/me/repohub-notes"));
        assertTrue(GitHubPaths.allowed("GET", "/repos/me/repohub-notes/git/trees/main?recursive=1"));
        assertFalse(GitHubPaths.allowed("PUT", "/repos/me/repohub-notes"));
        assertFalse(GitHubPaths.allowed("DELETE", "/repos/me/repohub-notes"));
        assertFalse(GitHubPaths.allowed("GET", "/repos/me/repohub-notes/git/trees/feature/x"));
        assertFalse(GitHubPaths.allowed("GET", "/repos/me/repohub-notes/git/blobs/abc123"));
        for (String bad : new String[] {"/repos/o/r/contents/README.md", "/repos/o/r/contents/.github/workflows/x.yml", "/repos/o/r/contents/boards/../x",
            "/repos/o/r/contents/boards/o/r/main/x.json", "/repos/o/r/contents/boards/o/r/main/.excalidraw", "/repos/o/r/contents/boards/o/r/x.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/sub/x.excalidraw", "/repos/o/r/contents/boards/o//main/x.excalidraw", "/repos/o/r/contents/boards/o/r/../x.excalidraw",
            "/repos/o/r/contents/boards/o/r/main/.hidden.excalidraw", "/repos/o/r/contents/boards/o/r/main/a b.excalidraw", "/repos/o/r/contents/notes/o/r/main/x.excalidraw",
            "/repos/o/r/contents"}) {
            assertFalse("PUT " + bad, GitHubPaths.allowed("PUT", bad));
            assertFalse("DELETE " + bad, GitHubPaths.allowed("DELETE", bad));
        }
        assertFalse(GitHubPaths.allowed("GET", "/repos/o/r/contents/README.md"));
        // Checked on the raw address before decoding: %25XX only, never an encoded slash or dot.
        String raw = "/repos/me/notes/contents/boards/octo/app/feature%252Fx%2520y/Plan%2520%2528v2%2529.excalidraw";
        assertEquals("/repos/me/notes/contents/boards/octo/app/feature%2Fx%20y/Plan%20%28v2%29.excalidraw", GitHubPaths.boardContentsPath(raw));
        assertNotNull(GitHubPaths.boardContentsPath("/repos/me/notes/contents/boards/o/r/%252E./x.excalidraw"));
        for (String bad : new String[] {"/repos/me/notes/contents/boards/o/r/feature%2Fx/x.excalidraw", "/repos/me/notes/contents/boards/o/r/%2e%2e/x.excalidraw",
            "/repos/me/notes/contents/boards/o/r/../x.excalidraw", "/repos/me/notes/contents/boards/o/r/main/.x.excalidraw", "/repos/me/notes/contents/boards/o/r//x.excalidraw",
            "/repos/me/notes/contents/boards/o/r/main/x%25.excalidraw", "/repos/me/notes/contents/boards/o/r/main/x%252.excalidraw", "/repos/me/notes/contents/boards/o/r/main/x%5Cy.excalidraw",
            "/repos/me/notes/contents/README.md", "/repos/me/notes/contents/boards/o/r/main/sub/x.excalidraw"}) {
            assertNull(bad, GitHubPaths.boardContentsPath(bad));
        }
        assertNull(GitHubPaths.decode("/repos/me/notes/contents/boards/o/r/feature%252Fx/x.excalidraw"));
    }

    @Test
    public void boardRequestsReachGitHubWithLargeBodies() {
        account.token = "ghp_secret";
        String raw = "gh/repos/me/notes/contents/boards/octo/app/feature%252Fx/Plan.excalidraw";
        StringBuilder big = new StringBuilder("{\"message\":\"Update\",\"content\":\"");
        for (int i = 0; i < 200_000; i++) {
            big.append('A');
        }
        big.append("\"}");
        assertEquals(200, backend.handle("PUT", raw, "", big.toString()).status);
        assertTrue(http.sent.get(0).startsWith("PUT https://api.github.com/repos/me/notes/contents/boards/octo/app/feature%252Fx/Plan.excalidraw token=ghp_secret"));
        assertEquals(413, backend.handle("POST", "gh/repos/o/r/issues", "", big.toString()).status);
        assertEquals(403, backend.handle("PUT", "gh/repos/me/notes/contents/README.md", "", "{}").status);
        assertEquals(403, backend.handle("PUT", "gh/repos/me/notes/contents/boards/o/r/feature%2Fx/x.excalidraw", "", "{}").status);
        assertEquals(1, http.sent.size());
    }

    @Test
    public void conditionalWritesTheTrashAndTheIndex() throws Exception {
        String f = "api/notes/file?repo=Octo%2FApp&branch=feature%2Fx&name=Plan";
        String one = "{\"elements\":[1]}";
        String two = "{\"elements\":[2]}";
        assertEquals(204, backend.handle("PUT", f + "&if=none", "", one).status);
        assertEquals(409, backend.handle("PUT", f + "&if=none", "", two).status);
        assertEquals("ce013625030ba8dba906f756967f9e9ca394464a", Notes.blobSha("hello\n".getBytes("UTF-8")));
        String sha = Notes.blobSha(one.getBytes("UTF-8"));
        assertEquals(409, backend.handle("PUT", f + "&if=0000000000000000000000000000000000000000", "", two).status);
        assertEquals(204, backend.handle("PUT", f + "&if=" + sha, "", two).status);
        JSONArray all = new JSONArray(backend.handle("GET", "api/notes/all", "", "").body);
        assertEquals(1, all.length());
        assertEquals("octo/app", all.getJSONObject(0).getString("repo"));
        assertEquals("feature/x", all.getJSONObject(0).getString("branch"));
        assertEquals(Notes.blobSha(two.getBytes("UTF-8")), all.getJSONObject(0).getString("sha"));
        assertEquals(409, backend.handle("DELETE", f + "&if=" + sha, "", "").status);
        assertEquals(204, backend.handle("DELETE", f + "&if=" + Notes.blobSha(two.getBytes("UTF-8")), "", "").status);
        assertEquals(404, backend.handle("GET", f, "", "").status);
        String day = java.time.LocalDate.now(java.time.ZoneOffset.UTC).toString();
        assertTrue("in the trash", new File(dir, "trash/" + day + "/octo/app/feature%2Fx/Plan.excalidraw").isFile());
        assertEquals("[]", backend.handle("GET", "api/notes/all", "", "").body);
        String i = "api/notes/index?repo=me%2Frepohub-notes";
        assertEquals(404, backend.handle("GET", i, "", "").status);
        assertEquals(204, backend.handle("PUT", i, "", "{\"version\":1}").status);
        assertEquals("{\"version\":1}", backend.handle("GET", i, "", "").body);
        assertEquals(400, backend.handle("PUT", i, "", "not json").status);
        assertEquals(400, backend.handle("GET", "api/notes/index?repo=..%2Fetc", "", "").status);
        assertEquals(204, backend.handle("DELETE", i, "", "").status);
        assertEquals(404, backend.handle("GET", i, "", "").status);
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
    public void conditionalReadsReachGitHubAndTheirTagsComeBack() {
        account.token = "ghp_secret";
        Backend.Reply first = backend.handle("GET", "gh/user/repos?per_page=100", "application/vnd.github+json", "", null);
        assertEquals(200, first.status);
        assertEquals("W/\"v1\"", first.etag);
        Backend.Reply again = backend.handle("GET", "gh/user/repos?per_page=100", "application/vnd.github+json", "", "W/\"v1\"");
        assertEquals(304, again.status);
        assertEquals("", again.body);
        assertEquals("GET https://api.github.com/user/repos?per_page=100 token=ghp_secret body= if-none-match=W/\"v1\"", http.sent.get(1));
        // Only entity tags on reads go on.
        backend.handle("GET", "gh/user", "", "", "*");
        backend.handle("GET", "gh/user", "", "", "\"a\"\r\nX-Evil: 1");
        backend.handle("PATCH", "gh/notifications/threads/12", "", "", "\"a\"");
        for (int i = 2; i < http.sent.size(); i++) {
            assertFalse(http.sent.get(i), http.sent.get(i).contains("if-none-match"));
        }
        assertEquals("W/\"a\", \"b\"", Backend.conditionalTag("GET", " W/\"a\", \"b\" "));
        assertNull(Backend.conditionalTag("GET", "abc"));
        assertNull(Backend.conditionalTag("GET", "\"" + "a".repeat(600) + "\""));
        assertNull(Backend.conditionalTag("GET", "\"\u00e9\""));
        assertNull(Backend.conditionalTag("POST", "\"a\""));
        // Refused paths still don't reach GitHub, conditional or not.
        int before = http.sent.size();
        assertEquals(403, backend.handle("GET", "gh/user/emails", "", "", "\"a\"").status);
        assertEquals(before, http.sent.size());
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
