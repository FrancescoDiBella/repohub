//! Diffs the app shows with rune-kit's `DiffView`: the local clone's changes since the last
//! commit, and the files of a pull request.
use rune_kit::diff::FileDiff;

use crate::github::PullFile;
use crate::local::Changes;

/// What the diff on screen is of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffOf {
    /// The local clone of a repository (its full name).
    Local(String),
    /// What merging a local branch would bring: the repository's full name and the branch.
    Merge(String, String),
}

/// The files of the local changes: `git diff HEAD`, then each new file as all-added.
pub fn local_files(c: &Changes) -> Vec<FileDiff> {
    let mut files = rune_kit::diff::parse(&c.patch);
    for (path, text) in &c.untracked {
        files.push(match text {
            Some(t) => FileDiff::between(None, Some(path.clone()), "", t),
            None => FileDiff::between(None, Some(path.clone()), "", "(binary or large file)\n"),
        });
    }
    files
}

/// The files of a pull request, from the patches GitHub gives without their headers.
pub fn pull_files(files: &[PullFile]) -> Vec<FileDiff> {
    let mut patch = String::new();
    for f in files {
        let old = f.previous.as_deref().unwrap_or(&f.path);
        patch.push_str(&format!("diff --git a/{old} b/{}\n", f.path));
        match f.status.as_str() {
            "added" => patch.push_str(&format!("new file mode 100644\n--- /dev/null\n+++ b/{}\n", f.path)),
            "removed" => patch.push_str(&format!("deleted file mode 100644\n--- a/{old}\n+++ /dev/null\n")),
            _ => {
                if f.previous.is_some() {
                    patch.push_str(&format!("rename from {old}\nrename to {}\n", f.path));
                }
                patch.push_str(&format!("--- a/{old}\n+++ b/{}\n", f.path));
            }
        }
        match &f.patch {
            Some(p) => {
                patch.push_str(p);
                if !p.ends_with('\n') {
                    patch.push('\n');
                }
            }
            None => patch.push_str("@@ -0,0 +1,1 @@\n+(GitHub shows no diff for this file: binary or too large)\n"),
        }
    }
    rune_kit::diff::parse(&patch)
}

/// A pull request's files for the sample data.
pub fn sample_pull_files() -> Vec<PullFile> {
    vec![
        PullFile { path: "src/scene.js".into(), previous: None, status: "modified".into(), patch: Some("@@ -10,7 +10,9 @@ export function scene(canvas) {\n   const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });\n-  renderer.setPixelRatio(1);\n+  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));\n   const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100);\n-  camera.position.set(0, 1, 4);\n+  camera.position.set(0, 1.2, 3.6);\n+  camera.lookAt(0, 0.6, 0);\n   return { renderer, camera };\n }".into()) },
        PullFile { path: "src/bunny.js".into(), previous: None, status: "added".into(), patch: Some("@@ -0,0 +1,6 @@\n+// A procedural bunny: spheres for the body, head and ears.\n+export function bunny(THREE) {\n+  const group = new THREE.Group();\n+  group.add(new THREE.Mesh(new THREE.SphereGeometry(0.5), new THREE.MeshStandardMaterial()));\n+  return group;\n+}".into()) },
    ]
}

/// Lines added and removed in all of `files`.
pub fn totals(files: &[FileDiff]) -> (usize, usize) {
    files.iter().fold((0, 0), |(a, d), f| (a + f.additions(), d + f.deletions()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_and_local_files() {
        let files = vec![
            PullFile { path: "src/a.rs".into(), previous: None, status: "modified".into(), patch: Some("@@ -1,2 +1,2 @@\n fn a() {}\n-let x = 1;\n+let x = 2;".into()) },
            PullFile { path: "new.md".into(), previous: None, status: "added".into(), patch: Some("@@ -0,0 +1,2 @@\n+# New\n+text".into()) },
            PullFile { path: "b.rs".into(), previous: Some("old_b.rs".into()), status: "renamed".into(), patch: None },
        ];
        let d = pull_files(&files);
        assert_eq!(d.iter().map(|f| f.path()).collect::<Vec<_>>(), ["src/a.rs", "new.md", "b.rs"]);
        assert_eq!((d[0].additions(), d[0].deletions()), (1, 1));
        assert_eq!((d[1].additions(), d[1].deletions()), (2, 0));
        assert_eq!(totals(&d).0, 4);
        let c = Changes {
            patch: "diff --git a/x.txt b/x.txt\n--- a/x.txt\n+++ b/x.txt\n@@ -1 +1,2 @@\n one\n+two\n".into(),
            untracked: vec![("n.txt".into(), Some("a\nb\n".into()))],
            ..Changes::default()
        };
        let l = local_files(&c);
        assert_eq!(l.iter().map(|f| (f.path().to_owned(), f.additions())).collect::<Vec<_>>(), [("x.txt".into(), 1), ("n.txt".into(), 2)]);
    }
}
