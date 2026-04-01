# Contribution Areas We Especially Welcome

This file is the repository-native contributor reference for where different
kinds of people can help LoongClaw most effectively.

The shorter public overview lives under
[`../../site/build-on-loongclaw/contribution-areas.mdx`](../../site/build-on-loongclaw/contribution-areas.mdx).
This file stays in the repository because contributors often still want the
deeper examples and source-level starting points behind that public overview.

LoongClaw needs more than one kind of contributor. We care about thoughtful software, durable
engineering, and a healthy community around the work.

You do not need to match every category below. If you bring depth, curiosity, clear communication,
and a willingness to take responsibility for what you ship, there is meaningful work here.

We also do not think meaningful contribution only looks like writing core runtime code. Design,
frontend polish, platform work, release hygiene, documentation, docs-site clarity, QA, and
community care all shape whether a project becomes genuinely useful and sustainable.

## Read This File When

- you want the repository-native contributor map instead of the shorter public
  overview
- you are deciding which contribution area best matches your strengths
- you want concrete repository surfaces to inspect before opening an issue or
  plan

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| get the short public overview first | [`../../site/build-on-loongclaw/contribution-areas.mdx`](../../site/build-on-loongclaw/contribution-areas.mdx) | this is the docs-site entrypoint |
| understand contribution workflow and validation rules | [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) | that file is the repository-native contributor guide |
| decide where your background can create the most leverage | this file | this page keeps the deeper area map and repository starting points |

## What We Value

- Ownership. We care about people who understand what they ship and can stand behind it.
- Taste. We value thoughtful decisions in product, interaction, API shape, documentation, and code.
- Care for others. Clear communication, good review habits, and patience with other contributors matter here.
- Long-term thinking. We prefer durable improvements over clever shortcuts that create future drag.
- Realistic contribution. You do not need to be full-time. A steady few hours each week can still matter.

## Where Your Strengths Can Help

| Area | We especially welcome | Why it matters here | Good starting points |
| --- | --- | --- | --- |
| Product and interface design | Designers with taste, product-minded frontend engineers, and people who care about clarity and trust | LoongClaw should feel calm, legible, and humane, not only technically capable | `README.md`, `docs/product-specs/`, onboarding, ask/chat/doctor flows |
| Frontend and interaction engineering | Engineers who can turn rough product surfaces into polished, reliable interfaces | The project needs strong UX and interaction work across CLI, channel flows, and future frontend surfaces | `docs/product-specs/`, `crates/app/src/channel/`, user-facing docs |
| Rust and systems engineering | Engineers with deep Rust experience and high standards for performance, memory safety, and clean architecture | The runtime lives or dies by its boundaries, reliability, and long-term maintainability | `crates/kernel/`, `crates/app/`, `docs/design-docs/`, `ARCHITECTURE.md` |
| Hardware, robotics, and embodied AI | Engineers who understand real-world devices, robotics constraints, or embodied assistant workflows | LoongClaw should grow beyond a narrow software-only shell and learn from real operational environments | `docs/ROADMAP.md`, channel and tool surfaces, device-oriented integrations |
| Cross-platform delivery | Engineers comfortable with macOS, Windows, Linux, Android, iOS, and packaging or install flows | A private assistant runtime only becomes practical when it is dependable across the systems people actually use | `scripts/install.sh`, `scripts/install.ps1`, `.github/workflows/release.yml`, platform support issues |
| Testing, QA, CI, and operations | People who enjoy validation, release hygiene, failure analysis, observability, and operational tooling | Strong product trust depends on fast feedback loops and boringly reliable releases | `.github/workflows/`, `scripts/`, `docs/releases/`, `docs/RELIABILITY.md` |
| Documentation and docs-site clarity | Contributors who like writing guides, improving examples, clarifying onboarding, or making the public docs easier to scan and trust | Good docs lower the barrier to contribution and real adoption, and the public docs surface should stay clear without turning the repository into a multi-locale markdown mirror | `README.md`, `README.zh-CN.md`, `CONTRIBUTING.md`, `docs/`, `site/`, issue templates |
| Community care | People who communicate well, enjoy helping others, and want to support triage, review, discussion, and community health | Open source becomes sustainable when contributors feel welcomed, respected, and unblocked | GitHub Discussions, issue triage, PR review, `CODE_OF_CONDUCT.md`, contributor docs |

## How To Join In

- If you already know what you want to work on, open or join the relevant Issue and link your plan.
- If you want to take on a large feature or architecture change, start with an Issue or Discussion first so maintainers can help shape scope early.
- If your strengths are docs, docs-site editing, QA, support, or community work, those are first-class contributions here, not second-tier work.
- If you would rather start with a direct introduction, email [contact@loongclaw.ai](mailto:contact@loongclaw.ai). A short note is enough. You do not need a formal application.
- If you are unsure where to begin, open a Discussion or send that introduction email and we will help point you toward good starting areas.

## A Short Introduction That Helps

If you email us, it is especially helpful to include:

- where you are based or what time zone you usually work in
- your strongest skills or the kinds of problems you are best at
- the area you would most like to own or help push forward
- what you hope LoongClaw could become, or what part of the project excites you
- roughly how much time or energy you expect to contribute
- any links to GitHub, past work, writing, design, demos, or projects you want us to see

That does not need to be long. A thoughtful, honest introduction is much more useful than a formal
pitch.

## Do Not Use This File For

- maintainer-only GitHub intake rules that belong in
  `github-collaboration.md`
- private planning bundles or backlog studies that do not belong in the OSS
  repository
- replacing the shorter public docs-site contribution overview when that page
  is enough
