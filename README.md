# Shoebox — a decentralized photos app, built on top of iroh

Your photos live on *your* devices — phone, laptop, a box under the desk — and sync
directly between them (and the people you share with) over [iroh](https://iroh.computer)
peer-to-peer connections. No account, no cloud bucket, no server that owns your library.

## What's in this folder

```
photos/
├── README.md                  ← you are here
├── docs/
│   ├── 01-vision.md           ← product principles, who it's for, non-goals
│   ├── 02-features.md         ← base feature set (MVP) + later phases
│   ├── 03-architecture.md     ← how features map onto iroh (endpoints, blobs, docs, gossip)
│   ├── 04-open-questions.md   ← decisions log + what's still open
│   └── 05-prototype-plan.md   ← crate layout, LibraryStore trait, keeper CLI, Tauri shell, milestones
└── design/
    ├── README.md              ← screen inventory + design notes
    └── mockups/
        ├── index.html         ← open this: links to every screen
        ├── assets/            ← shared CSS + JS for the mockups (placeholder photos etc.)
        ├── 01-onboarding.html
        ├── 02-library.html
        ├── 03-viewer.html
        ├── 04-albums-sharing.html
        ├── 05-devices.html
        └── 06-storage-settings.html
```

## Reviewing

1. Read `docs/02-features.md` — the feature list is numbered (`F1.2` etc.) so you can
   comment on items by ID.
2. Open `design/mockups/index.html` in a browser. Every screen is tagged with the
   feature IDs it covers.
3. Collect decisions in `docs/04-open-questions.md`.
4. Technical plan: `docs/05-prototype-plan.md`.

```bash
open design/mockups/index.html
```
