// Tests for parallel multi-instance and multi-agent Antigravity support (#429, #835).
// Verifies that multiple concurrent Antigravity sessions (CLI, IDE, Standalone, or multiple repos)
// get their own dedicated pills, independent mini-Mochi states, and diff isolation without crosstalk.

import { afterEach, beforeEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { calls, emit, sent } from "./tauri.mjs";
import { registerHookHandlers } from "../src/island/hooks.ts";
import { DEFAULT_SETTINGS, State } from "../src/core/state.ts";
import { setLanguage } from "../src/i18n/i18n.ts";
import { codeFor } from "../src/views/session.ts";

let asked = [];
const island = {
  alert: (view) => asked.push(`alert:${view}`),
  setView: (view) => asked.push(`setView:${view}`),
  reveal: () => asked.push("reveal"),
  dropPin: () => asked.push("dropPin"),
};
registerHookHandlers(island);

const hook = (payload) => emit("hook", payload);
const task = (id) => State.tasks.find((t) => t.id === id);

beforeEach(() => {
  setLanguage("en");
  mock.timers.enable({ apis: ["setTimeout"] });
  asked = [];
  calls.length = 0;
  State.tasks = [];
  State.focusId = null;
  State.mode = "hidden";
  State.view = "overview";
  State.paused = false;
  State.isPinned = false;
  State.pendingApproval = null;
  State.settings = { ...DEFAULT_SETTINGS };
  State.loadIntegrationTasks();
});

afterEach(() => {
  mock.timers.runAll();
  mock.timers.reset();
});

test("two concurrent Antigravity sessions in different projects get separate pills and mini-Mochis", () => {
  // Session 1 starts in repo-a
  hook({
    hook_event_name: "SessionStart",
    session_id: "sess-cli-1",
    coucou_agent: "antigravity",
    cwd: "C:\\Code\\repo-a",
    session_title: "repo-a",
  });
  hook({
    hook_event_name: "PreToolUse",
    session_id: "sess-cli-1",
    coucou_agent: "antigravity",
    tool_name: "run_command",
    tool_input: { CommandLine: "cargo test" },
  });

  const task1 = task("agent_antigravity");
  assert.ok(task1, "session 1 occupies base Antigravity pill");
  assert.equal(task1.sessionId, "sess-cli-1");
  assert.equal(task1.state, "working");
  assert.equal(task1.steps.at(-1), "Runs · cargo test");

  // Session 2 starts concurrently in repo-b with a different session ID
  hook({
    hook_event_name: "SessionStart",
    session_id: "sess-ide-2",
    coucou_agent: "antigravity",
    cwd: "C:\\Code\\repo-b",
    session_title: "repo-b",
    platform: "ide",
  });
  hook({
    hook_event_name: "PreToolUse",
    session_id: "sess-ide-2",
    coucou_agent: "antigravity",
    tool_name: "view_file",
    tool_input: { TargetFile: "C:\\Code\\repo-b\\src\\main.rs" },
  });

  const task2 = task("agent_antigravity:sess-ide-2");
  assert.ok(task2, "session 2 gets a dedicated concurrent session pill");
  assert.equal(task2.sessionId, "sess-ide-2");
  assert.equal(task2.state, "working");
  assert.equal(task2.steps.at(-1), "Reads · main.rs");
  assert.equal(task2.name, "Antigravity · [IDE] repo-b");

  // Verify both tasks coexist in State.tasks
  assert.ok(State.tasks.some((t) => t.id === "agent_antigravity"));
  assert.ok(State.tasks.some((t) => t.id === "agent_antigravity:sess-ide-2"));

  // Verify codeFor maintains independent code records for both sessions
  const code1 = codeFor("agent_antigravity");
  const code2 = codeFor("agent_antigravity:sess-ide-2");
  assert.ok(code1, "session 1 has its own code record");
  assert.ok(code2, "session 2 has its own code record");
  assert.notEqual(code1, code2, "code records must be distinct");
});

test("stopping one Antigravity session does not remove or affect the sibling session", () => {
  // Setup two active concurrent sessions
  hook({
    hook_event_name: "PreToolUse",
    session_id: "sess-1",
    coucou_agent: "antigravity",
    cwd: "C:\\Code\\repo-1",
    session_title: "repo-1",
    tool_name: "Bash",
    tool_input: { command: "build" },
  });
  hook({
    hook_event_name: "PreToolUse",
    session_id: "sess-2",
    coucou_agent: "antigravity",
    cwd: "C:\\Code\\repo-2",
    session_title: "repo-2",
    tool_name: "Bash",
    tool_input: { command: "test" },
  });

  assert.ok(task("agent_antigravity"));
  assert.ok(task("agent_antigravity:sess-2"));

  // Session 1 finishes its turn
  hook({
    hook_event_name: "Stop",
    session_id: "sess-1",
    coucou_agent: "antigravity",
    last_assistant_message: "Build complete",
  });

  assert.equal(task("agent_antigravity").state, "finished");
  assert.equal(task("agent_antigravity:sess-2").state, "working", "session 2 is still working");

  // Advance time past 5.2s stop timer
  mock.timers.tick(5500);

  // Session 1 is removed from external tasks, session 2 survives and is still working!
  assert.equal(task("agent_antigravity"), undefined, "session 1 pill removed after stop timer");
  const remaining = task("agent_antigravity:sess-2");
  assert.ok(remaining, "session 2 pill persists unaffected");
  assert.equal(remaining.state, "working");
});

test("surface tags like antigravity-cli or platform: cli are cleanly parsed into pill names", () => {
  hook({
    hook_event_name: "PreToolUse",
    coucou_agent: "antigravity-cli",
    session_id: "cli-99",
    cwd: "C:\\Code\\tools",
    session_title: "tools",
    tool_name: "Bash",
    tool_input: { command: "go" },
  });

  const t = task("agent_antigravity-cli");
  assert.ok(t);
  assert.equal(t.name, "Antigravity (CLI) · tools");
  assert.equal(t.color, "#E879F9");
});
