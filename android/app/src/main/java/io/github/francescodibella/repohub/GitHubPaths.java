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
                    if (rest.length == 1 && java.util.Arrays.asList("contents", "readme", "languages", "commits", "branches", "issues").contains(rest[0])) {
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
                // Creating an issue, a pull request or a branch; commenting; reviewing a pull request.
                return repo && ((rest.length == 1 && (rest[0].equals("issues") || rest[0].equals("pulls")))
                    || (rest.length == 2 && rest[0].equals("git") && rest[1].equals("refs"))
                    || (rest.length == 3 && number(rest[1]) && ((rest[0].equals("issues") && rest[2].equals("comments")) || (rest[0].equals("pulls") && (rest[2].equals("reviews") || rest[2].equals("comments"))))));
            case "PATCH":
                // Closing or reopening an issue; marking one notification as read.
                return (repo && rest.length == 2 && rest[0].equals("issues") && number(rest[1]))
                    || (seg.length == 3 && seg[0].equals("notifications") && seg[1].equals("threads") && number(seg[2]));
            default:
                return false;
        }
    }

    private static boolean starred(String[] seg) {
        return seg.length == 4 && seg[0].equals("user") && seg[1].equals("starred") && !seg[2].isEmpty() && !seg[3].isEmpty();
    }
}
