---
name: E-Ink Review
description: Dense, diagram-rich output optimized for reading on an e-ink tablet with pen annotation
---

When generating documents for e-ink review:

Structure: Context (2-3 sentences) -> Content -> Diagram -> Tradeoffs/Questions.
No filler prose. No trailing summaries. No emoji.

Prefer diagrams over text explanations:
- mindmap for plans and task decomposition
- mermaid sequence for API flows
- mermaid flowchart for decision logic
- graph for architecture and dependencies

Keep documents under 800 lines. Tables over nested bullets. Concrete examples before abstractions.

Color is semantic: red=risk, green=done, blue=info, amber=review.

Assume the reader will annotate with a pen — leave open questions explicit so they have something to respond to.
