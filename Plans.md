# Plans.md - Task Tracking

> **Project**: local-takkie
> **Last updated**: 2026-10-08
>
> **Source of truth:** GitHub Issues and the
> [Project board](https://github.com/users/martcpp/projects/3). This file only
> mirrors the tickets currently being worked on. Use the issue number plus the
> roadmap id (for example `#29 E1.1`). Remove tasks from here once their PR is
> merged.

---

## In Progress

<!-- Add tasks with cc:wip here. -->

- [ ] #150 E3.7: cargo xtask release <version> `cc:wip`

---

## Up Next

<!-- Add tasks with cc:todo here. Only the next few tickets of the current milestone. -->

- [ ] #51 E3.6: Release v0.1.0 `cc:todo`

---

## Done (waiting for owner)

<!-- Add tasks with cc:done here; remove once the PR is merged. -->

(none)

---

## Status Marker Legend

| Marker | Meaning |
|--------|---------|
| `pm:requested` | Requested by the owner |
| `cc:todo` | Not started |
| `cc:wip` | In progress |
| `cc:done` | Done, waiting for the owner to confirm |
| `pm:approved` | Confirmed by the owner |
| `blocked` | Blocked; write the reason next to the task |

Optional syntax:

```markdown
- [ ] #59 E5.2a: Packet header encoder `cc:todo` depends:#58
- [ ] #97 E7.3b: IPv4 address pick `cc:todo` [P]
```

`depends:#N` lists dependencies; `[P]` marks tasks that can run in parallel.
