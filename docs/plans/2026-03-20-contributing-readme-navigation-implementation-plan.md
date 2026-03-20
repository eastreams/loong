# Contributing Guide Visibility Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the contributing guide easier to discover from the top README navigation, clarify the project's stance on responsible agent-assisted coding and harness engineering in `CONTRIBUTING.md`, and align the Chinese README with the Chinese SVG assets.

**Architecture:** Keep the change docs-only. Reuse the existing top-of-file anchor navigation in both READMEs, add no new sections beyond contributor-facing documentation copy, and point the Chinese README at the already-shipped `-zh` SVG assets.

**Tech Stack:** Markdown, repository docs conventions

---

## Task 1: Update top-level README navigation

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`

**Step 1: Add the contributing link to the top navigation**

Insert a `Contributing` / `贡献` entry into the existing top anchor navigation so readers can jump to the contributing section without scrolling through the full document.

**Step 2: Keep anchors aligned with existing section ids**

Use the existing `#contributing` anchor in `README.md` and the Chinese README's contribution section anchor instead of creating duplicate sections.

**Step 3: Verify the navigation remains readable**

Check the surrounding separator style and ordering so the new link feels like part of the existing compact nav block.

## Task 2: Clarify contribution philosophy

**Files:**
- Modify: `CONTRIBUTING.md`

**Step 1: Add contributor guidance about agent-assisted coding**

Document that the project sees human + agent collaboration as an important long-term engineering direction and welcomes responsible use of AI tooling.

**Step 2: Preserve accountability expectations**

State clearly that contributors must still understand what they submit and remain responsible for correctness, tradeoffs, and operational consequences.

**Step 3: Mention harness engineering**

Explain that maintainers are actively improving harness-engineering workflows in the repository so contributors can use agents more effectively and more safely.

## Task 3: Align localized visuals

**Files:**
- Modify: `README.zh-CN.md`

**Step 1: Replace English SVG references with Chinese assets**

Switch the current Chinese README image references to `assets/readme/loongclaw-positioning-map-zh.svg` and `assets/readme/loongclaw-foundation-diagram-zh.svg`.

**Step 2: Verify the alt text and placement stay unchanged**

Keep the current Chinese alt text and section structure intact so only the localized asset changes.

## Task 4: Verify docs-only results

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `CONTRIBUTING.md`

**Step 1: Review the final diff**

Run: `git diff -- README.md README.zh-CN.md CONTRIBUTING.md docs/plans/2026-03-20-contributing-readme-navigation-implementation-plan.md`

Expected: Only the navigation link additions, SVG swaps, contributor-policy copy, and the plan document appear.

**Step 2: Optional markdown sanity check**

Run: `sed -n '35,55p' README.md && sed -n '35,55p' README.zh-CN.md`

Expected: The top nav blocks show the new contributor links in the intended position.

**Step 3: Commit**

```bash
git add README.md README.zh-CN.md CONTRIBUTING.md docs/plans/2026-03-20-contributing-readme-navigation-implementation-plan.md
git commit -m "docs: surface contributing guide and agent workflow"
```
