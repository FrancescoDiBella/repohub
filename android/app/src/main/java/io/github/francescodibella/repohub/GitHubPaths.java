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

    private static boolean name(String s) {
        return !s.isEmpty() && !s.startsWith(".") && s.length() <= 100 && s.matches("[A-Za-z0-9._-]+");
    }

    /** One board segment as the app writes it (decoded once): [A-Za-z0-9_.-] and %XX, no leading dot. */
    private static boolean boardSegment(String s) {
        return !s.isEmpty() && !s.startsWith(".") && s.length() <= 255 && s.matches("([A-Za-z0-9_.-]|%[0-9A-Fa-f]{2})+");
    }

    /** {@code contents/boards/<owner>/<repo>/<branch>/<name>.excalidraw}: the only files board sync touches. */
    private static boolean boardFile(String[] rest) {
        if (rest.length != 6 || !rest[0].equals("contents") || !rest[1].equals("boards")) {
            return false;
        }
        for (int i = 2; i < 6; i++) {
            if (!boardSegment(rest[i])) {
                return false;
            }
        }
        return rest[5].endsWith(".excalidraw") && rest[5].length() > ".excalidraw".length();
    }

    /**
     * A board's contents path, checked on the raw address before anything is decoded, as
     * repohub-server's {@code board_contents_path}: each board segment is [A-Za-z0-9_.-] or
     * %25XX (the app's %XX, itself encoded). No "..", empty segment, encoded slash or dot.
     * Returns the path decoded once, or null.
     */
    static String boardContentsPath(String raw) {
        if (!raw.startsWith("/")) {
            return null;
        }
        String[] seg = raw.substring(1).split("/", -1);
        if (seg.length != 9 || !seg[0].equals("repos") || !name(seg[1]) || !name(seg[2]) || !seg[3].equals("contents") || !seg[4].equals("boards")) {
            return null;
        }
        for (int i = 5; i < 9; i++) {
            if (seg[i].isEmpty() || seg[i].startsWith(".") || !seg[i].matches("([A-Za-z0-9_.-]|%25[0-9A-Fa-f]{2})+")) {
                return null;
            }
        }
        return raw.replace("%25", "%");
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
                    // Board sync: the notes repository itself, its file list, and its boards.
                    if (rest.length == 0 || boardFile(rest)) {
                        return true;
                    }
                    if (rest.length == 3 && rest[0].equals("git") && rest[1].equals("trees") && name(rest[2])) {
                        return true;
                    }
                    if (rest.length == 2 && ((rest[0].equals("actions") && rest[1].equals("runs")) || (rest[0].equals("stats") && rest[1].equals("participation")))) {
                        return true;
                    }
                    if (rest.length == 1 && java.util.Arrays.asList("contents", "readme", "languages", "commits", "branches", "issues").contains(rest[0])) {
                        return true;
                    }
                    // A repository's labels, assignable people and milestones (triage pickers).
                    if (rest.length == 1 && (rest[0].equals("labels") || rest[0].equals("assignees") || rest[0].equals("milestones"))) {
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
                // Board sync writes one board, under boards/ and ending in .excalidraw, nothing else.
                return (repo && boardFile(rest)) || starred(seg);
            case "DELETE":
                // Taking back a review request and deleting a board file are the only DELETEs in a repository.
                return requestedReviewers(repo, rest) || (repo && boardFile(rest)) || starred(seg);
            case "POST":
                // Creating an issue, a pull request or a branch; commenting; reviewing a pull request;
                // asking for a review; re-running a workflow.
                return repo && ((rest.length == 1 && (rest[0].equals("issues") || rest[0].equals("pulls")))
                    || requestedReviewers(repo, rest)
                    || (rest.length == 2 && rest[0].equals("git") && rest[1].equals("refs"))
                    || (rest.length == 3 && number(rest[1]) && ((rest[0].equals("issues") && rest[2].equals("comments")) || (rest[0].equals("pulls") && (rest[2].equals("reviews") || rest[2].equals("comments")))))
                    || (rest.length == 4 && rest[0].equals("actions") && rest[1].equals("runs") && number(rest[2]) && (rest[3].equals("rerun") || rest[3].equals("rerun-failed-jobs"))));
            case "PATCH":
                // Closing or reopening an issue, and editing its labels, assignees, milestone, title and
                // description (pull requests too); marking one notification as read.
                return (repo && rest.length == 2 && rest[0].equals("issues") && number(rest[1]))
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

    /** `/repos/{o}/{r}/pulls/{digits}/requested_reviewers`, nothing under it. */
    private static boolean requestedReviewers(boolean repo, String[] rest) {
        return repo && rest.length == 3 && rest[0].equals("pulls") && number(rest[1]) && rest[2].equals("requested_reviewers");
    }
}
