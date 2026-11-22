package io.github.francescodibella.repohub;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.time.LocalDate;
import java.time.ZoneOffset;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The drawings, kept as .excalidraw files in the app's private storage. Same layout, names and
 * answers as repohub-server's notes (server/src/notes.rs): conditional writes ({@code if=}),
 * deletes into {@code trash/<date>/} for 30 days, every drawing with its git blob id
 * ({@code api/notes/all}) and board sync's index ({@code api/notes/index}).
 */
final class Notes {
    static final long MAX_BYTES = 25L << 20;
    static final long MAX_INDEX_BYTES = 4L << 20;
    private static final AtomicLong TMP = new AtomicLong(1);
    /** One read-compare-write at a time. */
    private static final Object LOCK = new Object();
    private final File data;
    private final File root;

    Notes(File dataDir) {
        this.data = dataDir;
        this.root = new File(dataDir, "notes");
    }

    /** The git blob id of a file's content (what GitHub calls its sha). */
    static String blobSha(byte[] content) {
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-1");
            d.update(("blob " + content.length + "\0").getBytes(StandardCharsets.UTF_8));
            d.update(content);
            StringBuilder out = new StringBuilder();
            for (byte b : d.digest()) {
                out.append(String.format(Locale.ROOT, "%02x", b & 0xff));
            }
            return out.toString();
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static String today(long offsetDays) {
        return LocalDate.now(ZoneOffset.UTC).minusDays(offsetDays).toString();
    }

    private static byte[] readOrNull(File f) {
        try {
            return Files.readAllBytes(f.toPath());
        } catch (IOException e) {
            return null;
        }
    }

    private static File[] sorted(File dir) {
        File[] kids = dir.listFiles();
        if (kids == null) {
            return new File[0];
        }
        Arrays.sort(kids);
        return kids;
    }

    /** Moves a drawing to trash/<today>/…, a taken name getting a number; drops trash older than 30 days. */
    void trash(File file) throws IOException {
        String rel = root.toPath().relativize(file.toPath()).toString();
        File day = new File(new File(data, "trash"), today(0));
        File dest = new File(day, rel);
        String stem = file.getName().substring(0, file.getName().length() - ".excalidraw".length());
        for (int n = 2; dest.exists(); n++) {
            dest = new File(dest.getParentFile(), stem + " " + n + ".excalidraw");
        }
        File parent = dest.getParentFile();
        if (!parent.isDirectory() && !parent.mkdirs()) {
            throw new IOException("the trash folder can't be created");
        }
        Files.move(file.toPath(), dest.toPath(), StandardCopyOption.ATOMIC_MOVE);
        String cutoff = today(30);
        for (File d : sorted(new File(data, "trash"))) {
            if (d.getName().matches("\\d{4}-\\d{2}-\\d{2}") && d.getName().compareTo(cutoff) < 0) {
                deleteTree(d);
            }
        }
    }

    private static void deleteTree(File f) {
        for (File k : sorted(f)) {
            deleteTree(k);
        }
        f.delete();
    }

    /** Every drawing, as {@code GET api/notes/all} lists it. */
    String listAll() {
        StringBuilder out = new StringBuilder("[");
        for (File o : sorted(root)) {
            for (File r : sorted(o)) {
                for (File b : sorted(r)) {
                    for (File f : sorted(b)) {
                        if (!f.isFile() || !f.getName().endsWith(".excalidraw")) {
                            continue;
                        }
                        byte[] bytes = readOrNull(f);
                        if (bytes == null) {
                            continue;
                        }
                        String name = f.getName().substring(0, f.getName().length() - ".excalidraw".length());
                        if (out.length() > 1) {
                            out.append(',');
                        }
                        out.append("{\"repo\":").append(Backend.jsonString(unsegment(o.getName()) + "/" + unsegment(r.getName())))
                            .append(",\"branch\":").append(Backend.jsonString(unsegment(b.getName())))
                            .append(",\"name\":").append(Backend.jsonString(unsegment(name)))
                            .append(",\"sha\":\"").append(blobSha(bytes)).append('"')
                            .append(",\"updated\":").append(f.lastModified() / 1000)
                            .append(",\"size\":").append(bytes.length).append('}');
                    }
                }
            }
        }
        return out.append(']').toString();
    }

    /** {@code api/notes/index?repo=owner/name}: board sync's index for that notes repository. */
    private Backend.Reply index(String method, Map<String, String> q, String body) {
        String repo = q.getOrDefault("repo", "");
        if (!validRepo(repo)) {
            return Backend.Reply.text(400, "repo (owner/name) is required");
        }
        File file = new File(new File(data, "notes-sync"), segment(repo.toLowerCase(Locale.ROOT)) + ".json");
        switch (method) {
            case "GET": {
                byte[] b = readOrNull(file);
                return b == null ? Backend.Reply.text(404, "Nothing synced yet") : Backend.Reply.json(200, new String(b, StandardCharsets.UTF_8));
            }
            case "PUT": {
                byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
                if (bytes.length > MAX_INDEX_BYTES || !body.startsWith("{")) {
                    return Backend.Reply.text(400, "Expected a sync index (JSON, 4 MB at most)");
                }
                try {
                    write(file, bytes);
                    return Backend.Reply.text(204, "");
                } catch (IOException e) {
                    return Backend.Reply.text(500, "Couldn't save the sync index: " + e.getMessage());
                }
            }
            case "DELETE":
                file.delete();
                return Backend.Reply.text(204, "");
            default:
                return Backend.Reply.text(405, "Method not allowed");
        }
    }

    /** Writes beside and moves in place, so a crash never leaves half a file. */
    private static void write(File file, byte[] bytes) throws IOException {
        File folder = file.getParentFile();
        File tmp = new File(folder, file.getName() + ".tmp." + TMP.getAndIncrement());
        try {
            if (!folder.isDirectory() && !folder.mkdirs()) {
                throw new IOException("the folder can't be created");
            }
            try (FileOutputStream out = new FileOutputStream(tmp)) {
                out.write(bytes);
                out.getFD().sync();
            }
            Files.move(tmp.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (IOException e) {
            tmp.delete();
            throw e;
        }
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

    /** Answers `method` on `url` ("api/notes?…", "api/notes/file?…", "api/notes/all", "api/notes/index?…"). */
    Backend.Reply handle(String method, String url, String body) {
        Map<String, String> q = query(url);
        int qAt = url.indexOf('?');
        String route = qAt < 0 ? url : url.substring(0, qAt);
        if (route.equals("api/notes/all")) {
            return method.equals("GET") ? Backend.Reply.json(200, listAll()) : Backend.Reply.text(405, "Method not allowed");
        }
        if (route.equals("api/notes/index")) {
            synchronized (LOCK) {
                return index(method, q, body);
            }
        }
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
        // `if`: only when the file still has this blob id ("none": only when there is none).
        String cond = q.get("if");
        synchronized (LOCK) {
            if (cond != null && (method.equals("PUT") || method.equals("DELETE"))) {
                byte[] now = readOrNull(file);
                String sha = now == null ? null : blobSha(now);
                boolean same = cond.equals("none") ? sha == null : cond.equals(sha);
                if (!same) {
                    return now == null && method.equals("DELETE") ? Backend.Reply.text(404, "No such drawing") : Backend.Reply.text(409, "The drawing changed meanwhile");
                }
            }
            return file(method, folder, file, name, body);
        }
    }

    private Backend.Reply file(String method, File folder, File file, String name, String body) {
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
                try {
                    write(file, bytes);
                    return Backend.Reply.text(204, "");
                } catch (IOException e) {
                    return Backend.Reply.text(500, "Couldn't save the drawing: " + e.getMessage());
                }
            }
            case "DELETE":
                if (!file.isFile()) {
                    return Backend.Reply.text(404, "No such drawing");
                }
                try {
                    trash(file);
                    return Backend.Reply.text(204, "");
                } catch (IOException e) {
                    return Backend.Reply.text(500, "Couldn't delete the drawing: " + e.getMessage());
                }
            default:
                return Backend.Reply.text(405, "Method not allowed");
        }
    }

    long totalBytes() {
        return size(root);
    }
}
