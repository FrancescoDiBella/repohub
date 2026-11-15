package io.github.francescodibella.repohub;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The drawings, kept as .excalidraw files in the app's private storage. Same layout, names and
 * answers as repohub-server's notes (server/src/notes.rs).
 */
final class Notes {
    static final long MAX_BYTES = 25L << 20;
    private static final AtomicLong TMP = new AtomicLong(1);
    private final File root;

    Notes(File dataDir) {
        this.root = new File(dataDir, "notes");
    }

    static Map<String, String> query(String url) {
        Map<String, String> out = new HashMap<>();
        int q = url.indexOf('?');
        if (q < 0) {
            return out;
        }
        for (String pair : url.substring(q + 1).split("&")) {
            if (pair.isEmpty()) {
                continue;
            }
            int eq = pair.indexOf('=');
            String k = eq < 0 ? pair : pair.substring(0, eq);
            String v = eq < 0 ? "" : pair.substring(eq + 1);
            try {
                out.put(URLDecoder.decode(k, "UTF-8"), URLDecoder.decode(v, "UTF-8"));
            } catch (Exception e) {
                // A malformed pair is left out; the caller then misses a required value.
            }
        }
        return out;
    }

    private static boolean part(String p) {
        return !p.isEmpty() && p.length() <= 100 && !p.startsWith(".") && p.matches("[A-Za-z0-9._-]+");
    }

    static boolean validRepo(String r) {
        String[] parts = r.split("/", -1);
        return parts.length == 2 && part(parts[0]) && part(parts[1]);
    }

    static boolean validBranch(String b) {
        if (b.isEmpty() || b.getBytes(StandardCharsets.UTF_8).length > 255) {
            return false;
        }
        for (int i = 0; i < b.length(); i++) {
            if (Character.isISOControl(b.charAt(i))) {
                return false;
            }
        }
        return true;
    }

    static boolean validName(String n) {
        if (n.isEmpty() || n.getBytes(StandardCharsets.UTF_8).length > 80 || n.startsWith(".")) {
            return false;
        }
        return n.codePoints().allMatch(c -> Character.isLetterOrDigit(c) || " _-.()".indexOf(c) >= 0);
    }

    /** One file-name segment: [A-Za-z0-9_.-] kept (no leading dot), the rest percent-encoded. */
    static String segment(String s) {
        StringBuilder out = new StringBuilder();
        for (byte b : s.getBytes(StandardCharsets.UTF_8)) {
            int c = b & 0xff;
            boolean keep = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-' || (c == '.' && out.length() > 0);
            if (keep) {
                out.append((char) c);
            } else {
                out.append(String.format(Locale.ROOT, "%%%02X", c));
            }
        }
        return out.toString();
    }

    private static String unsegment(String s) {
        try {
            return URLDecoder.decode(s.replace("+", "%2B"), "UTF-8");
        } catch (Exception e) {
            return s;
        }
    }

    File folder(String repo, String branch) {
        int slash = repo.indexOf('/');
        String owner = repo.substring(0, slash).toLowerCase(Locale.ROOT);
        String name = repo.substring(slash + 1).toLowerCase(Locale.ROOT);
        return new File(new File(new File(root, segment(owner)), segment(name)), segment(branch));
    }

    private static long size(File f) {
        if (f.isDirectory()) {
            long total = 0;
            File[] kids = f.listFiles();
            if (kids != null) {
                for (File k : kids) {
                    total += size(k);
                }
            }
            return total;
        }
        return f.length();
    }

    /** Answers `method` on `url` ("api/notes?…" or "api/notes/file?…"). */
    Backend.Reply handle(String method, String url, String body) {
        Map<String, String> q = query(url);
        String repo = q.getOrDefault("repo", "");
        String branch = q.getOrDefault("branch", "");
        if (!validRepo(repo) || !validBranch(branch)) {
            return Backend.Reply.text(400, "repo (owner/name) and branch are required");
        }
        File folder = folder(repo, branch);
        int at = url.indexOf('?');
        String path = at < 0 ? url : url.substring(0, at);
        if (path.equals("api/notes")) {
            if (!method.equals("GET")) {
                return Backend.Reply.text(405, "Method not allowed");
            }
            List<File> files = new ArrayList<>();
            File[] kids = folder.listFiles();
            if (kids != null) {
                for (File k : kids) {
                    if (k.isFile() && k.getName().endsWith(".excalidraw")) {
                        files.add(k);
                    }
                }
            }
            files.sort((a, b) -> Long.compare(b.lastModified(), a.lastModified()));
            StringBuilder out = new StringBuilder("[");
            for (File f : files) {
                String name = f.getName();
                name = unsegment(name.substring(0, name.length() - ".excalidraw".length()));
                if (out.length() > 1) {
                    out.append(',');
                }
                out.append("{\"name\":").append(Backend.jsonString(name))
                    .append(",\"updated\":").append(f.lastModified() / 1000)
                    .append(",\"size\":").append(f.length()).append('}');
            }
            return Backend.Reply.json(200, out.append(']').toString());
        }
        if (!path.equals("api/notes/file")) {
            return Backend.Reply.text(404, "Not found");
        }
        String name = q.getOrDefault("name", "");
        if (!validName(name)) {
            return Backend.Reply.text(400, "Invalid name: use letters, digits, spaces, _ - . ( ), at most 80 characters");
        }
        File file = new File(folder, segment(name) + ".excalidraw");
        switch (method) {
            case "GET":
                try {
                    return Backend.Reply.json(200, new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8));
                } catch (IOException e) {
                    return Backend.Reply.text(404, "No such drawing");
                }
            case "PUT": {
                byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
                if (bytes.length > MAX_BYTES) {
                    return Backend.Reply.text(413, "Drawing too large (25 MB at most)");
                }
                if (!body.startsWith("{")) {
                    return Backend.Reply.text(400, "Expected an Excalidraw JSON document");
                }
                File tmp = new File(folder, segment(name) + ".tmp." + TMP.getAndIncrement());
                try {
                    if (!folder.isDirectory() && !folder.mkdirs()) {
                        throw new IOException("the folder can't be created");
                    }
                    try (FileOutputStream out = new FileOutputStream(tmp)) {
                        out.write(bytes);
                        out.getFD().sync();
                    }
                    Files.move(tmp.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
                    return Backend.Reply.text(204, "");
                } catch (IOException e) {
                    tmp.delete();
                    return Backend.Reply.text(500, "Couldn't save the drawing: " + e.getMessage());
                }
            }
            case "DELETE":
                return file.delete() ? Backend.Reply.text(204, "") : Backend.Reply.text(404, "No such drawing");
            default:
                return Backend.Reply.text(405, "Method not allowed");
        }
    }

    long totalBytes() {
        return size(root);
    }
}
