# State machines

RepoHub's states are named types in [`src/state.rs`](../src/state.rs) (and two local ones next
to their code). A state that makes no sense can't be written, and every change goes through one
method. Run with `REPOHUB_TRACE=1` to see each change on stderr:

```
state: shown "nothing" -> "saved copy"
state: time "saved copy shown" -> "4 ms"
state: status "starting" -> "loading"
state: time "first frame" -> "90 ms"
state: page Overview -> Repo(0)
state: tab Code -> Notes
```

`time` lines say how long since the process started: the saved copy shown, the first frame, the
profile and repositories shown, the load done, the copy written.

## Data source: `Status`

```mermaid
stateDiagram-v2
    [*] --> Starting: account configured
    [*] --> Sample: no account / demo
    Starting --> Loading: refresh()
    Loading --> Ready: data arrived
    Loading --> Failed: error
    Ready --> Loading: refresh()
    Failed --> Loading: Retry
    Sample --> Starting: Connect (Settings)
    Ready --> Sample: Sign out
```

`refresh()` only starts from `Starting`, `Ready` or `Failed` (`Status::can_load`), so two loads
never run together.

What is on screen meanwhile (`cache::Fresh`, beside `Status`):

| | Nothing saved (first start) | Saved copy (`from_copy`) | Loaded in this session |
| --- | --- | --- | --- |
| `Starting`, `Loading` | skeletons, then the profile and repositories with the other lists loading (`partial`) | the copy, "Updating…" | the lists, "Updating…" |
| `Ready` | — | — (the copy is replaced) | "Updated 3 minutes ago" |
| `Failed` | the error with **Retry** | the copy, the error and "The lists show what was saved 2 hours ago" | the lists and the error |

A load's results carry the account generation (`load_gen`): a load or a part of one that started
before a sign-out or an account switch is dropped, and the copy it would have saved is never
written (`cache::forget_all` stops writes still on their way).

## Something fetched once: `Load<T>`

Repository details and branch lists. A missing map entry means "not asked yet".

```mermaid
stateDiagram-v2
    [*] --> Loading: first open
    Loading --> Ready
    Loading --> Failed
```

The drawings list uses `Refresh` instead (`Loading`, `Fresh`, `Failed`): it keeps its last items
on screen while it reloads.

## Saving a drawing: `Save`

```mermaid
stateDiagram-v2
    [*] --> Saved: new drawing
    [*] --> Loading: open a drawing
    Loading --> Saved: loaded
    Loading --> Failed: can't read
    Saved --> SavingClean: edit (write starts)
    SavingClean --> Saved: written
    SavingClean --> Failed: error
    SavingClean --> SavingDirty: edit (waits)
    SavingDirty --> SavingClean: written (writes again)
    Failed --> SavingClean: edit (write starts)
```

One write runs at a time. Edits made meanwhile set `dirty` and are written next, so an older
write can't land after a newer one. Closing the editor writes whatever is still pending.

A write merges with the file if it changed underneath (sync brought another device's edits):
`notes::save` reads, merges (`sync::merge`) and writes in one step, and the editor takes in the
merge (`Board::merge_json`).

## Boards on every device: `BoardSync` and `NotesRepoCheck`

```mermaid
stateDiagram-v2
    [*] --> Off: no notes repository (or sample data, no token)
    Off --> Ready: Use it (Settings)
    Ready --> Running: Notes opened / a save, a few seconds later / Sync now / refresh
    Running --> Done: run ended
    Running --> Failed: offline, refused, rate-limited
    Done --> Running: the same triggers
    Failed --> Running: Retry
    Done --> Off: Stop syncing
    Failed --> Off: Stop syncing
```

A trigger while a run is `Running` sets `again`, and one more run follows. A run's result is
dropped if a newer run started or the account changed (`sync.run`, `load_gen`). The boards
still to send (`sync.to_send`) are counted live, so a save after the run shows at once.

Choosing the repository in Settings:

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Checking: Use it
    Checking --> Idle: private and writable (chosen)
    Checking --> ConfirmPublic: public
    ConfirmPublic --> Idle: Use it anyway (chosen) / Cancel
    Checking --> Failed: missing, no access, not writable
    Failed --> Checking: Use it
```

What a run does with each board is a pure function, `sync::decide`, of the local side and the
remote side (unchanged, changed, deleted, new) against the index; `sync::tests` checks all 16
cells.

## The canvas: `board::Mode`

```mermaid
stateDiagram-v2
    Idle --> Creating: press (shape tool)
    Creating --> Idle: release (kept, or dropped if tiny)
    Idle --> Moving: press on a shape (Select)
    Moving --> Idle: release
    Idle --> Typing: press (Text)
    Typing --> Idle: Enter / another press / another tool
```

## Navigation: `Page` and `RepoTab`

```mermaid
stateDiagram-v2
    direction LR
    Overview --> Repos
    Repos --> Repo: open(i)
    state Repo {
        [*] --> Code
        Code --> Notes: set_tab
        Notes --> Terminal: set_tab
    }
    Repo --> Repos: Back
```

`go(page)` and `set_tab(i, tab)` are the only ways to move. Leaving a section closes the open
drawing (after writing it) and drops pending confirmations and error messages; entering Notes
loads its lists, entering Terminal opens the shell when the clone is known.

## The Commits section: `commits::Commits`

```mermaid
stateDiagram-v2
    [*] --> History: set_tab(Commits), a branch picked
    History --> Commit: a row
    History --> Compare: Compare
    Compare --> Commit: a row of the result
    Commit --> Commit: a parent (Back returns to the child)
    Commit --> History: Back / Esc / ⌘[ (opened from the history)
    Commit --> Compare: Back / Esc / ⌘[ (opened from the comparison)
    Compare --> History: × / Esc / ⌘[
```

The history (`History`), the open commit (`Open`) and the comparison (`Compare`) each hold a
`Load` and the epoch of the request they wait for; an answer with another epoch (another branch,
commit or comparison, or a reload since) is dropped, as is one for another account (`load_gen`).
Leaving the section closes the commit and the comparison; the history stays for next time.

## The Releases section: `releases::Releases`

```mermaid
stateDiagram-v2
    [*] --> List: set_tab(Releases)
    List --> Form: New release / Draft a release / Edit (a draft)
    Form --> Asking: Publish release
    Asking --> Form: Cancel / Esc
    Asking --> Sending: Publish
    Form --> Sending: Save draft
    Sending --> List: saved (the list and the header follow)
    Sending --> Form: GitHub refuses (what was typed stays)
    Form --> List: × / Cancel / Esc
    List --> DraftAsking: Publish (on a draft)
    DraftAsking --> List: Cancel / Esc, or published
```

The list (`List`) holds a `Load` and the epoch of the request it waits for; an answer for another
repository, a reload since, or another account is dropped. Generate notes waits the same way, and
its answer waits for a yes (`Form::offered`) when the notes field has something you typed.
Publishing happens only from the question (`Form::confirm`, `Releases::confirm`); leaving the
section drops the question and keeps the form. Each repository's latest release (`latest`) is
read when its page opens, for the header.

## The Terminal tab: `terminal_page::Panel`

```mermaid
stateDiagram-v2
    [*] --> Missing: no local clone known
    [*] --> Found: clone found on disk
    Missing --> Running: use a folder / clone
    Found --> Running: Open terminal
    Running --> Exited: the shell ends
    Exited --> Running: Start again
```

## Smaller ones

| Type | States | Notes |
| --- | --- | --- |
| `Theme` | `System`, `Light`, `Dark` | `toggled()` flips what is on screen; saved by name |
| `RepoFilter` | `All`, `Public`, `Private`, `Fork` | `matches(private, fork)` |
| `RepoSort` | `Recent`, `Stars`, `Name` | |
| `PullState` | `Open`, `Draft`, `Merged`, `Closed` | also the Pull requests page's filter |
| `Account` | `Locked`, `SignedOut`, `SignedIn` | `App::account()`; drives the form in Settings |
| `Toast` | none → shown (`Done` / `Problem`) → none | its timer hides only its own message |
| `Facet` | `Labels`, `Assignees`, `Milestone`, `Reviewers` | the open triage picker (one at a time, or none); Apply sends one `triage::Change` |
| `markdown::Block` | `Text`, `Fence`, `Comment` | the README pass, line by line |
| `releases::TagUse` | `Invalid`, `Taken`, `Existing`, `New` | what the form's tag does: refused, another release's, used as it is, or created from the target branch |
| `releases::NotesJob` | `Idle`, `Asking`, `Failed` | Generate notes |

## For agents and scripts

- `-- page repo tab notes` opens a section by name (numbers still work).
- `REPOHUB_TRACE=1` prints every transition.
- Every control has a stable key (`tabs`, `theme`, `acct-save`, `tool-Rectangle`…) for `--script`.
