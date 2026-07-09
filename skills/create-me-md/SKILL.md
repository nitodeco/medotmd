---
name: create-medotmd
description: Use when the user asks to create, draft, refine, initialize, or improve a ME.md (medotmd) file, personal AI profile, identity prompt, agent memory profile, or medotmd identity file.
---

# Create ME.md

Create a useful `ME.md`: a portable identity profile that helps AI agents understand who the user is.

## Workflow

1. Inspect context first.
   - If a `ME.md` exists, read it before asking questions.
   - If this is inside a medotmd repo or install, read `examples/jane-doe/ME.md` when available for the expected shape.
   - If the user asks about CLI setup or installation, read [medotmd-cli.md](references/medotmd-cli.md).
   - Completion: existing profile, available example, and relevant CLI context are accounted for.

2. Interview for unknowns.
   - Ask one precise, simple question at a time.
   - Don't proactively ask for sensitive information.
   - Stop interviewing once the profile can be useful without guessing.
   - Completion: every important section below has either user-provided facts, safe inferred defaults, or an explicit omission.

3. Draft the profile.
   - Write clear Markdown with simple headings and short bullets.
   - Use first person when writing as the user.
   - Keep durable preferences; avoid temporary tasks, mood, and project trivia unless the user explicitly wants them.
   - Separate facts from instructions. Facts describe the user; instructions tell agents how to work.
   - Completion: the draft is copyable as a complete `ME.md` without placeholders.

4. Review and write.
   - Call out any assumptions in one short note.
   - Ask before overwriting an existing `ME.md`.
   - If writing locally, create parent folders as needed and preserve existing content unless the user approved replacement.
   - Completion: the user has either received the finished Markdown or the file has been written where requested.

## Interview Map

Cover these areas. Skip anything already known from the existing profile or current context.

- Identity: name, timezone, languages, high-level location if useful.
- Work: role, domain, current responsibilities, collaboration style.
- Personal context: durable life context that affects assistance, without sensitive detail.
- Tools: primary machine, operating system, editor, terminal, package managers, project tools, AI agents; infer what you can using CLI.
- Preferences: communication, scheduling, travel, units, dates, recommendation style.
- Coding if applicable: languages, frameworks, testing expectations, dependency posture, code review style.
- Interests: topics the user wants agents to remember for recommendations, examples, and tone.
- Boundaries: actions that need approval, things not to do

Important: Dont't ask questions that you already know the answer to through existing memory and context.

## Good ME.md Shape

Prefer this simple structure unless the user needs something different:

```md
## About me

## Personal preferences

## Work preferences

## Tools I use most

## Boundaries
```

## Quality Bar

A strong `ME.md` is:

- Useful to both personal AI and coding agents.
- Specific enough to change agent behavior.
- Safe to keep in a plain local Markdown file.
- Short enough to be read every session.

Reject or revise content that is:

- Secret, credential-like, or too private for a reusable plain-text file.
- So generic it would apply to almost anyone.
- A temporary todo list pretending to be identity.
- A system prompt full of broad commands without personal context.
