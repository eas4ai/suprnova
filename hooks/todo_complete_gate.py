#!/usr/bin/env python3
"""
Shared todo-complete production gate for Claude Code and Codex.

Behavior:
- Reads hook JSON from stdin.
- Finds the latest visible todo list from hook input, transcript JSONL, or the last assistant message.
- If the current todo list is complete, triggers a one-time BEST_PRACTICES.md final gate.
- For Stop/PostToolUse-style hooks, returns JSON `decision: block` so the agent performs one more review/revision pass.
- Optionally runs a verification command when BEST_PRACTICES_VERIFY_COMMAND is set.

Optional environment variables:
- BEST_PRACTICES_VERIFY_COMMAND: shell command to run when todos are complete, e.g. "npm test && npm run lint".
- BEST_PRACTICES_VERIFY_TIMEOUT: timeout in seconds for that command. Default: 120.
- BEST_PRACTICES_GATE_STATE_DIR: directory for one-time session markers. Default: system temp dir.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from typing import Any

MARKER = "BEST_PRACTICES_FINAL_GATE_TRIGGERED"

# Package version. BEST_PRACTICES.md carries the same number on its
# "Ruleset version" line; the test suite fails when the two disagree.
__version__ = "1.1.0"
RULESET_VERSION_RE = re.compile(r"^Ruleset version (\d+\.\d+\.\d+)\b", re.MULTILINE)

# Checked, in order, only after the walk up from cwd finds no repo-local copy.
# Lets a single global install cover projects that never ran the installer.
GLOBAL_BEST_PRACTICES_PATHS = (
    Path.home() / ".claude" / "BEST_PRACTICES.md",
    Path.home() / ".codex" / "BEST_PRACTICES.md",
)
RELEASE_GATE_RULE = (
    "You are delivering production software. Do not deliver work you know to be deficient. "
    "Review the implementation against every rule in BEST_PRACTICES.md and answer honestly: "
    "are you satisfied it follows this ruleset, or would you make revisions? "
    "If revisions are needed, make them before delivering."
)

COMPLETE_STATUSES = {
    "complete",
    "completed",
    "done",
    "closed",
    "fixed",
    "resolved",
    "checked",
    "success",
    "succeeded",
    "passed",
}

INCOMPLETE_STATUSES = {
    "pending",
    "in_progress",
    "in-progress",
    "in progress",
    "todo",
    "to_do",
    "open",
    "not_started",
    "not started",
    "started",
    "active",
    "blocked",
    "failed",
    "failing",
}

DECISION_BLOCK_EVENTS = {
    "Stop",
    "SubagentStop",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "UserPromptSubmit",
    "UserPromptExpansion",
    "ConfigChange",
    "PreCompact",
}

EXIT_2_BLOCK_EVENTS = {"TaskCompleted", "TaskCreated", "TeammateIdle"}


def main() -> int:
    hook_input = read_hook_input()
    event_name = str(hook_input.get("hook_event_name") or "")
    cwd = resolve_cwd(hook_input)

    todos = find_current_todos(hook_input)
    if not todos or not all_todos_complete(todos):
        return 0

    # Stop hooks may fire again after this hook has already asked the agent to continue.
    # Avoid a loop by allowing that second pass to stop.
    if bool(hook_input.get("stop_hook_active")):
        return 0

    verification_failure = run_optional_verification(cwd)
    session_key = session_state_key(hook_input, cwd)
    if verification_failure is None and gate_already_triggered(session_key):
        return 0

    reason = build_gate_reason(todos, cwd, verification_failure)
    if verification_failure is None:
        mark_gate_triggered(session_key)

    if event_name in EXIT_2_BLOCK_EVENTS:
        print(reason, file=sys.stderr)
        return 2

    # Claude Code and Codex both support top-level decision:block for Stop-style continuation.
    # Claude Code also supports it for PostToolUse, which is how we catch TodoWrite completion.
    if event_name in DECISION_BLOCK_EVENTS or not event_name:
        print(json.dumps({"decision": "block", "reason": reason}))
        return 0

    # Unknown event: avoid breaking the session, but show a useful message if stdout is consumed.
    print(reason)
    return 0


def read_hook_input() -> dict[str, Any]:
    raw = sys.stdin.read()
    if not raw.strip():
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return {"_raw_stdin": raw}
    return parsed if isinstance(parsed, dict) else {"_stdin": parsed}


def resolve_cwd(hook_input: dict[str, Any]) -> Path:
    cwd_value = hook_input.get("cwd") or os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()
    try:
        return Path(str(cwd_value)).expanduser().resolve()
    except OSError:
        return Path.cwd()


def find_current_todos(hook_input: dict[str, Any]) -> list[dict[str, str]]:
    """Return the latest visible todo list, normalized to content/status dicts."""
    direct_lists = extract_todo_lists(hook_input)
    if direct_lists:
        return direct_lists[-1]

    transcript_path = hook_input.get("transcript_path")
    if transcript_path:
        path = Path(str(transcript_path)).expanduser()

        # Two transcript shapes exist. TodoWrite-style agents log a whole
        # `todos` array per update, so the latest array IS the current list.
        # Claude Code's Task* tools do not: TaskCreate carries one subject,
        # TaskUpdate carries {taskId, status}, and the periodic task_reminder
        # snapshot is only accurate as of where it sits in the transcript.
        #
        # So the replay runs FIRST and unconditionally. It consumes snapshots
        # as checkpoints and then applies every later event, which is the only
        # reading that is correct at the end of the file. Ordering this the
        # other way round -- or gating it on the event being Task*-shaped --
        # lets a stale snapshot win on `Stop`, where the payload carries no
        # task fields, and the gate then silently never fires.
        #
        # The replay returns [] for a session that used neither, so TodoWrite
        # transcripts still fall through to the array scan.
        for extractor in (replay_task_tool_state, latest_transcript_todo_list):
            todos = extractor(path)
            if todos:
                return todos

    last_message = str(hook_input.get("last_assistant_message") or "")
    text_todos = extract_todos_from_text(last_message)
    if text_todos:
        return text_todos

    return []


def latest_transcript_todo_list(path: Path) -> list[dict[str, str]]:
    lists = extract_todos_from_transcript(path)
    return lists[-1] if lists else []


# `Task #12 created successfully: Subject text`
TASK_CREATED_PATTERN = re.compile(r"Task #(\d+) created successfully:\s*(.*)")


def replay_task_tool_state(path: Path) -> list[dict[str, str]]:
    """Rebuild the current task list by replaying the whole transcript.

    Claude Code's Task* tools never emit the current list in the hook payload,
    so it has to be reconstructed from three kinds of evidence, in file order:

    * `TaskCreate` adds a task (its id arrives in the tool_result, not the input)
    * `TaskUpdate` changes one task's status
    * a `task_reminder` snapshot restates the whole roster

    A snapshot is treated as a checkpoint that reseeds the roster rather than as
    the answer: it is authoritative *at its position* and picks up tasks created
    before this transcript began, but any event after it still wins. Reading the
    last snapshot as the final answer is what made the gate miss completed work.

    Streams line by line rather than reading a fixed tail -- truncating the head
    would drop TaskCreate events and silently hide their tasks, which could turn
    an unfinished list into a falsely-complete one.
    """
    if not path.exists() or not path.is_file():
        return []

    tasks: dict[str, dict[str, str]] = {}
    order: list[str] = []
    pending_creates: dict[str, str] = {}

    def record(task_id: str, content: str, status: str) -> None:
        if task_id not in tasks:
            order.append(task_id)
            tasks[task_id] = {"content": content, "status": status}
        else:
            tasks[task_id]["status"] = status
            if content:
                tasks[task_id]["content"] = content

    try:
        with path.open("r", encoding="utf-8", errors="replace") as handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue

                content_blocks = (obj.get("message") or {}).get("content")
                if not isinstance(content_blocks, list):
                    continue

                for block in content_blocks:
                    if not isinstance(block, dict):
                        continue

                    if block.get("type") == "task_reminder":
                        reseed_from_snapshot(block.get("content"), tasks, order)
                        continue

                    if block.get("type") == "tool_use":
                        name = block.get("name")
                        tool_input = block.get("input")
                        if not isinstance(tool_input, dict):
                            continue
                        if name == "TaskCreate":
                            pending_creates[str(block.get("id"))] = str(
                                tool_input.get("subject") or "unnamed task"
                            )
                        elif name == "TaskUpdate":
                            task_id = str(tool_input.get("taskId") or "")
                            status = str(tool_input.get("status") or "")
                            if task_id and status:
                                # An update for an id we never saw created can only
                                # happen if the transcript is partial; keep it (with
                                # its real status) so an unfinished task still blocks.
                                record(task_id, tasks.get(task_id, {}).get("content", f"task #{task_id}"), status)

                    elif block.get("type") == "tool_result":
                        tool_use_id = str(block.get("tool_use_id") or "")
                        if tool_use_id not in pending_creates:
                            continue
                        match = TASK_CREATED_PATTERN.search(tool_result_text(block))
                        if match:
                            subject = match.group(2).strip() or pending_creates[tool_use_id]
                            record(match.group(1), subject, "pending")
                        pending_creates.pop(tool_use_id, None)
    except OSError:
        return []

    # Deleted tasks are neither complete nor incomplete -- leaving them in
    # would wedge all_todos_complete() at False forever.
    return [
        {"content": tasks[task_id]["content"], "status": tasks[task_id]["status"]}
        for task_id in order
        if normalize_status(tasks[task_id]["status"]) != "deleted"
    ]


def reseed_from_snapshot(
    entries: Any,
    tasks: dict[str, dict[str, str]],
    order: list[str],
) -> None:
    """Fold a task_reminder snapshot into the replay state, in place.

    Snapshots carry the full roster, so they recover tasks whose TaskCreate
    predates this transcript. They are applied where they appear, so later
    TaskUpdate events overwrite them rather than the other way round.
    """
    if not isinstance(entries, list):
        return

    for entry in entries:
        if not isinstance(entry, dict):
            continue
        task_id = str(entry.get("id") or entry.get("taskId") or "")
        status = str(entry.get("status") or "")
        if not task_id or not status:
            continue
        # `subject` is the name; `description` is the body. Using the body
        # buries the list in prose in the gate message.
        content = str(entry.get("subject") or entry.get("content") or f"task #{task_id}")
        if task_id not in tasks:
            order.append(task_id)
        tasks[task_id] = {"content": content, "status": status}


def tool_result_text(block: dict[str, Any]) -> str:
    """Flatten a tool_result's content, which may be a string or block list."""
    content = block.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return " ".join(
            str(part.get("text") or "") for part in content if isinstance(part, dict)
        )
    return ""


def extract_todos_from_transcript(path: Path) -> list[list[dict[str, str]]]:
    if not path.exists() or not path.is_file():
        return []

    try:
        text = read_file_tail(path, max_bytes=2_000_000)
    except OSError:
        return []

    found: list[list[dict[str, str]]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            text_todos = extract_todos_from_text(line)
            if text_todos:
                found.append(text_todos)
            continue
        found.extend(extract_todo_lists(obj))

    if not found:
        text_todos = extract_todos_from_text(text)
        if text_todos:
            found.append(text_todos)

    return found


def read_file_tail(path: Path, max_bytes: int) -> str:
    size = path.stat().st_size
    with path.open("rb") as handle:
        if size > max_bytes:
            handle.seek(size - max_bytes)
            handle.readline()  # discard partial line
        data = handle.read()
    return data.decode("utf-8", errors="replace")


def extract_todo_lists(obj: Any) -> list[list[dict[str, str]]]:
    found: list[list[dict[str, str]]] = []

    def walk(value: Any) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key in {"todos", "todo_list", "tasks", "task_list", "plan"} and isinstance(child, list):
                    normalized = normalize_todo_list(child)
                    if normalized:
                        found.append(normalized)
                walk(child)
        elif isinstance(value, list):
            normalized = normalize_todo_list(value)
            if normalized:
                found.append(normalized)
            for item in value:
                walk(item)

    walk(obj)
    return found


def normalize_todo_list(items: list[Any]) -> list[dict[str, str]]:
    if not items:
        return []

    normalized: list[dict[str, str]] = []
    status_count = 0
    for item in items:
        if not isinstance(item, dict):
            return []

        status_raw = first_present(
            item,
            "status",
            "state",
            "todo_status",
            "task_status",
            "checked",
            "complete",
            "completed",
        )
        if isinstance(status_raw, bool):
            status = "completed" if status_raw else "pending"
        else:
            status = normalize_status(str(status_raw or ""))

        # Order matters: Claude Code's task objects carry both `subject` (the
        # short title) and `description` (the full brief). Reading description
        # first buries the completed-list summary in paragraphs of prose.
        content = str(
            first_present(
                item,
                "content",
                "task",
                "todo",
                "title",
                "subject",
                "description",
                "text",
                "name",
            )
            or "unnamed todo"
        ).strip()

        if status:
            status_count += 1
        normalized.append({"content": content, "status": status})

    if status_count == 0:
        return []

    # Avoid false positives from unrelated arrays by requiring every item to expose status-like state.
    if status_count != len(normalized):
        return []

    return normalized


def first_present(item: dict[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in item:
            return item[key]
    return None


def normalize_status(value: str) -> str:
    return value.strip().lower().replace("-", "_").replace(" ", "_")


def extract_todos_from_text(text: str) -> list[dict[str, str]]:
    if not text:
        return []

    checkbox_pattern = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+\[([ xX])\]\s+(.+?)\s*$", re.MULTILINE)
    matches = checkbox_pattern.findall(text)
    if matches:
        return [
            {"content": content.strip(), "status": "completed" if mark.lower() == "x" else "pending"}
            for mark, content in matches
        ]

    # Conservative fallback for simple textual todo summaries.
    status_line_pattern = re.compile(
        r"^\s*(?:[-*+]|\d+[.)])\s+(completed|complete|done|pending|in[_ -]?progress|open|blocked)\s*[:\-–]\s+(.+?)\s*$",
        re.IGNORECASE | re.MULTILINE,
    )
    matches = status_line_pattern.findall(text)
    if matches:
        return [{"content": content.strip(), "status": normalize_status(status)} for status, content in matches]

    return []


def all_todos_complete(todos: list[dict[str, str]]) -> bool:
    if not todos:
        return False

    for todo in todos:
        status = normalize_status(todo.get("status", ""))
        if status in INCOMPLETE_STATUSES:
            return False
        if status not in COMPLETE_STATUSES:
            return False
    return True


def run_optional_verification(cwd: Path) -> str | None:
    command = os.environ.get("BEST_PRACTICES_VERIFY_COMMAND")
    if not command:
        return None

    try:
        timeout = int(os.environ.get("BEST_PRACTICES_VERIFY_TIMEOUT", "120"))
    except ValueError:
        timeout = 120

    try:
        completed = subprocess.run(
            command,
            cwd=str(cwd),
            shell=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        output = trim_output((exc.stdout or "") if isinstance(exc.stdout, str) else "")
        return f"Verification command timed out after {timeout}s: `{command}`\n{output}".strip()
    except OSError as exc:
        return f"Verification command could not start: `{command}`\n{exc}"

    if completed.returncode != 0:
        output = trim_output(completed.stdout or "")
        return f"Verification command failed with exit code {completed.returncode}: `{command}`\n{output}".strip()

    return None


def trim_output(output: str, limit: int = 4000) -> str:
    output = output.strip()
    if len(output) <= limit:
        return output
    return "... [output trimmed] ...\n" + output[-limit:]


def build_gate_reason(todos: list[dict[str, str]], cwd: Path, verification_failure: str | None) -> str:
    todo_summary = "\n".join(f"- {todo['content']} [{todo['status']}]" for todo in todos[:25])
    if len(todos) > 25:
        todo_summary += f"\n- ... {len(todos) - 25} more completed todo(s)"

    best_practices_path = find_best_practices_path(cwd)
    if best_practices_path:
        ruleset_version = read_ruleset_version(cwd / best_practices_path)
        ruleset_label = f"ruleset {ruleset_version}" if ruleset_version else "ruleset unversioned, older than 1.1.0"
        path_note = f"Open and apply `{best_practices_path}` ({ruleset_label}; hook {__version__})."
    else:
        path_note = "Find, read, and apply `BEST_PRACTICES.md`."

    verification_note = ""
    if verification_failure:
        verification_note = (
            "\n\nThe optional verification command failed. Fix the failure before delivering:\n"
            f"{verification_failure}\n"
        )

    return f"""{MARKER}

The current todo list appears complete, so run the production completion gate before delivering final output.

Completed todo list detected:
{todo_summary}

{path_note} Inspect the implementation, tests, edge cases, build/lint/type status, error handling, security, data handling, documentation, user-visible behavior, and the writing itself (simple technical English, rule 14). If anything would require revision, keep working and make the revisions now.{verification_note}
Release gate (rule 13):
\"{RELEASE_GATE_RULE}\"

When satisfied, deliver only after you can honestly answer that no revisions are needed under this ruleset. Include what changed and what verification passed. Do not claim unrun verification passed.
""".strip()


def read_ruleset_version(path: Path) -> str | None:
    """Version from the "Ruleset version X.Y.Z" line under the ruleset title.

    None means the copy predates versioning (1.1.0) or cannot be read; the
    gate runs either way.
    """
    try:
        head = path.read_text(encoding="utf-8", errors="replace")[:2000]
    except OSError:
        return None
    match = RULESET_VERSION_RE.search(head)
    return match.group(1) if match else None


def find_best_practices_path(cwd: Path) -> str | None:
    """Locate the ruleset: nearest repo copy wins, then the global fallback.

    Walking up from cwd first means a repository that ships its own tailored
    ruleset overrides the global one. The fallback exists so the gate can
    still name a concrete file when installed globally, in a project that
    never ran the installer.
    """
    for candidate in [cwd, *cwd.parents]:
        path = candidate / "BEST_PRACTICES.md"
        if path.exists():
            try:
                return str(path.relative_to(cwd)) if path.is_relative_to(cwd) else str(path)
            except ValueError:
                return str(path)

    for fallback in GLOBAL_BEST_PRACTICES_PATHS:
        if fallback.exists():
            return str(fallback)

    return None


def session_state_key(hook_input: dict[str, Any], cwd: Path) -> str:
    session_id = str(hook_input.get("session_id") or "no-session")
    transcript_path = str(hook_input.get("transcript_path") or "no-transcript")
    raw = f"{session_id}\n{cwd}\n{transcript_path}\n{MARKER}".encode("utf-8", errors="replace")
    return hashlib.sha256(raw).hexdigest()


def state_dir() -> Path:
    base = os.environ.get("BEST_PRACTICES_GATE_STATE_DIR")
    if base:
        path = Path(base).expanduser()
    else:
        path = Path(tempfile.gettempdir()) / "best-practices-agent-gates"
    path.mkdir(parents=True, exist_ok=True)
    return path


def gate_already_triggered(key: str) -> bool:
    return (state_dir() / f"{key}.json").exists()


def mark_gate_triggered(key: str) -> None:
    path = state_dir() / f"{key}.json"
    payload = {"marker": MARKER}
    try:
        path.write_text(json.dumps(payload), encoding="utf-8")
    except OSError:
        # If state cannot be written, prefer one extra quality pass over silent failure.
        pass


if __name__ == "__main__":
    raise SystemExit(main())
