# State machines

RepoHub's states are named types in [`src/state.rs`](../src/state.rs) (and two local ones next
to their code). A state that makes no sense can't be written, and every change goes through one
method. Run with `REPOHUB_TRACE=1` to see each change on stderr:

```
state: status "starting" -> "loading"
state: page Overview -> Repo(0)
state: tab Code -> Notes
```

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
| `markdown::Block` | `Text`, `Fence`, `Comment` | the README pass, line by line |

## For agents and scripts

- `-- page repo tab notes` opens a section by name (numbers still work).
- `REPOHUB_TRACE=1` prints every transition.
- Every control has a stable key (`tabs`, `theme`, `acct-save`, `tool-Rectangle`…) for `--script`.
