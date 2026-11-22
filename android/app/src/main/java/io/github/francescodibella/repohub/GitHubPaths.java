package io.github.francescodibella.repohub;

import java.io.ByteArrayOutputStream;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.ByteBuffer;
import java.util.Locale;

/**
 * Which GitHub API paths the app may call with the saved token. The same rules as
 * repohub-server's proxy (server/src/main.rs): keep the two in step.
 */
final class GitHubPaths {
    private GitHubPaths() {}

    /** The decoded path, or null when it could escape the allowlist (traversal, odd encodings). */
    static String decode(String rawPath) {
        String lower = rawPath.toLowerCase(Locale.ROOT);
        if (lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") || rawPath.indexOf('\\') >= 0 || hasControl(rawPath)) {
            return null;
        }
        String decoded = percentDecode(rawPath);
        if (decoded == null || decoded.indexOf('\\') >= 0 || decoded.indexOf('%') >= 0 || hasControl(decoded)) {
            return null;
        }
        String stripped = decoded;
        while (stripped.startsWith("/")) {
            stripped = stripped.substring(1);
        }
        if (stripped.isEmpty()) {
            return decoded;
        }
        for (String seg : stripped.split("/", -1)) {
            if (seg.isEmpty() || seg.matches("\\.+")) {
                return null;
            }
        }
        return decoded;
    }

    private static boolean hasControl(String s) {
        for (int i = 0; i < s.length(); i++) {
            if (Character.isISOControl(s.charAt(i))) {
                return true;
            }
        }
        return false;
    }

    private static String percentDecode(String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        ByteArrayOutputStream out = new ByteArrayOutputStream(b.length);
        for (int i = 0; i < b.length; i++) {
            if (b[i] == '%') {
                if (i + 2 >= b.length) {
                    return null;
                }
                int hi = Character.digit(b[i + 1], 16);
                int lo = Character.digit(b[i + 2], 16);
                if (hi < 0 || lo < 0) {
                    return null;
                }
                out.write(hi * 16 + lo);
                i += 2;
            } else {
                out.write(b[i]);
            }
        }
        try {
            return StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(out.toByteArray())).toString();
        } catch (CharacterCodingException e) {
            return null;
        }
    }

    private static boolean number(String s) {
        return !s.isEmpty() && s.matches("[0-9]+");
    }

    /** True when `method` on the decoded `path` (query allowed) is one the app uses. */
    static boolean allowed(String method, String path) {
        int q = path.indexOf('?');
        String p = q < 0 ? path : path.substring(0, q);
        String trimmed = p;
        while (trimmed.startsWith("/")) {
            trimmed = trimmed.substring(1);
        }
        String[] seg = trimmed.split("/", -1);
        boolean repo = seg.length >= 3 && seg[0].equals("repos") && !seg[1].isEmpty() && !seg[2].isEmpty();
        String[] rest = repo ? java.util.Arrays.copyOfRange(seg, 3, seg.length) : new String[0];
        switch (method) {
            case "GET":
                if (p.equals("/user") || p.equals("/user/repos") || p.equals("/user/starred") || p.equals("/user/orgs") || p.equals("/search/issues") || p.equals("/notifications")) {
                    return true;
                }
                if (seg.length == 2 && seg[0].equals("users") && !seg[1].isEmpty()) {
                    return true;
                }
                if (seg.length == 3 && seg[0].equals("users") && !seg[1].isEmpty() && (seg[2].equals("repos") || seg[2].equals("orgs"))) {
                    return true;
                }
                if (seg.length == 3 && seg[0].equals("orgs") && !seg[1].isEmpty() && seg[2].equals("repos")) {
                    return true;
                }
                if (repo) {
                    if (rest.length == 2 && ((rest[0].equals("actions") && rest[1].equals("runs")) || (rest[0].equals("stats") && rest[1].equals("participation")))) {
                        return true;
                    }
                    if (rest.length == 1 && java.util.Arrays.asList("contents", "readme", "languages", "commits", "branches", "issues", "releases", "tags").contains(rest[0])) {
                        return true;
                    }
                    // The latest release (not one release by id or tag).
                    if (rest.length == 2 && rest[0].equals("releases") && rest[1].equals("latest")) {
                        return true;
                    }
                    if (rest.length >= 4 && rest[0].equals("git") && rest[1].equals("ref") && rest[2].equals("heads")) {
                        for (int i = 3; i < rest.length; i++) {
                            if (rest[i].isEmpty()) {
                                return false;
                            }
                        }
                        return true;
                    }
                    if (rest.length == 3 && rest[0].equals("pulls") && number(rest[1]) && (rest[2].equals("files") || rest[2].equals("reviews") || rest[2].equals("comments"))) {
                        return true;
                    }
                    // One issue or pull request and its conversation.
                    if (rest.length == 2 && (rest[0].equals("issues") || rest[0].equals("pulls")) && number(rest[1])) {
                        return true;
                    }
                    if (rest.length == 3 && rest[0].equals("issues") && number(rest[1]) && rest[2].equals("comments")) {
                        return true;
                    }
                    if (rest.length == 4 && rest[0].equals("actions") && rest[1].equals("runs") && number(rest[2]) && rest[3].equals("jobs")) {
                        return true;
                    }
                    if (rest.length == 4 && rest[0].equals("actions") && rest[1].equals("jobs") && number(rest[2]) && rest[3].equals("logs")) {
                        return true;
                    }
                    // The checks on a pull request's last commit (a hex commit id).
                    if (rest.length == 3 && rest[0].equals("commits") && rest[1].matches("[0-9a-fA-F]{7,64}") && rest[2].equals("check-runs")) {
                        return true;
                    }
                    // One commit with its files, by its id only.
                    if (rest.length == 2 && rest[0].equals("commits") && rest[1].matches("[0-9a-fA-F]{7,64}")) {
                        return true;
                    }
                    // Two branches compared: `base...head`, each a branch name (which may hold `/`).
                    if (rest.length >= 2 && rest[0].equals("compare") && compareSpec(String.join("/", java.util.Arrays.copyOfRange(rest, 1, rest.length)))) {
                        return true;
                    }
                }
                return false;
            case "PUT":
                if (repo && rest.length == 3 && rest[0].equals("pulls") && number(rest[1]) && rest[2].equals("merge")) {
                    return true;
                }
                return starred(seg);
            case "DELETE":
                return starred(seg);
            case "POST":
                // Creating an issue, a pull request, a branch or a release (and its generated notes);
                // commenting; reviewing a pull request; re-running a workflow.
                return repo && ((rest.length == 1 && (rest[0].equals("issues") || rest[0].equals("pulls") || rest[0].equals("releases")))
                    || (rest.length == 2 && rest[0].equals("git") && rest[1].equals("refs"))
                    || (rest.length == 2 && rest[0].equals("releases") && rest[1].equals("generate-notes"))
                    || (rest.length == 3 && number(rest[1]) && ((rest[0].equals("issues") && rest[2].equals("comments")) || (rest[0].equals("pulls") && (rest[2].equals("reviews") || rest[2].equals("comments")))))
                    || (rest.length == 4 && rest[0].equals("actions") && rest[1].equals("runs") && number(rest[2]) && (rest[3].equals("rerun") || rest[3].equals("rerun-failed-jobs"))));
            case "PATCH":
                // Closing or reopening an issue; saving or publishing a draft release (by its number);
                // marking one notification as read.
                return (repo && rest.length == 2 && (rest[0].equals("issues") || rest[0].equals("releases")) && number(rest[1]))
                    || (seg.length == 3 && seg[0].equals("notifications") && seg[1].equals("threads") && number(seg[2]));
            default:
                return false;
        }
    }

    /** A branch name GitHub takes (the app's `github::valid_branch`). */
    static boolean branchName(String name) {
        if (name.isEmpty() || name.length() > 200 || name.startsWith("-") || name.startsWith("/") || name.startsWith(".")
            || name.endsWith("/") || name.endsWith(".") || name.endsWith(".lock")
            || name.contains("..") || name.contains("//") || name.contains("@{")) {
            return false;
        }
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            if (Character.isWhitespace(c) || Character.isISOControl(c) || "~^:?*[\\".indexOf(c) >= 0) {
                return false;
            }
        }
        return true;
    }

    /** `base...head` of a comparison: two branch names around the three dots. */
    static boolean compareSpec(String spec) {
        int at = spec.indexOf("...");
        return at >= 0 && branchName(spec.substring(0, at)) && branchName(spec.substring(at + 3));
    }

    private static boolean starred(String[] seg) {
        return seg.length == 4 && seg[0].equals("user") && seg[1].equals("starred") && !seg[2].isEmpty() && !seg[3].isEmpty();
    }
}
