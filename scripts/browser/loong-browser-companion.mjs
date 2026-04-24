#!/usr/bin/env node

import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import process from "node:process";

const COMPANION_VERSION = "0.1.0";
const AGENT_BROWSER_COMMAND_TIMEOUT_MS = 25000;

function decodeCommandOutput(value) {
  if (!value) {
    return "";
  }

  if (typeof value === "string") {
    return value;
  }

  return new TextDecoder("utf-8").decode(value);
}

function resolveEdgeExecutablePath() {
  if (process.env.AGENT_BROWSER_EXECUTABLE_PATH) {
    const normalizedEnvPath = normalizeKnownEdgePath(process.env.AGENT_BROWSER_EXECUTABLE_PATH);
    if (normalizedEnvPath) {
      return normalizedEnvPath;
    }

    return resolveWindowsShortPath(process.env.AGENT_BROWSER_EXECUTABLE_PATH) ??
      process.env.AGENT_BROWSER_EXECUTABLE_PATH;
  }

  if (process.platform !== "win32") {
    return null;
  }

  const candidates = [
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  ];
  const edgePath = candidates.find((candidate) => existsSync(candidate));
  if (!edgePath) {
    return null;
  }

  const normalizedEdgePath = normalizeKnownEdgePath(edgePath);
  if (normalizedEdgePath) {
    return normalizedEdgePath;
  }

  return resolveWindowsShortPath(edgePath) ?? edgePath;
}

function normalizeKnownEdgePath(path) {
  if (process.platform !== "win32") {
    return null;
  }

  const normalized = path.toLowerCase().replaceAll("/", "\\");
  if (normalized.endsWith("\\microsoft\\edge\\application\\msedge.exe")) {
    if (normalized.startsWith("c:\\program files (x86)\\")) {
      return "C:\\PROGRA~2\\MICROS~1\\Edge\\APPLIC~1\\msedge.exe";
    }

    if (normalized.startsWith("c:\\program files\\")) {
      return "C:\\PROGRA~1\\MICROS~1\\Edge\\APPLIC~1\\msedge.exe";
    }
  }

  return null;
}

function resolveWindowsShortPath(path) {
  if (process.platform !== "win32") {
    return null;
  }

  const escaped = path.replace(/"/g, '""');
  const result = spawnSync(
    "cmd.exe",
    ["/d", "/c", `for %I in ("${escaped}") do @echo %~sI`],
    {
      windowsHide: true,
    },
  );
  if ((result.status ?? 1) !== 0) {
    return null;
  }

  return (
    decodeCommandOutput(result.stdout)
      .split(/\r?\n/)
      .map((value) => value.trim())
      .find((value) => value.length > 0) ?? null
  );
}

function resolveAgentBrowserInvocation() {
  if (process.platform !== "win32") {
    return {
      command: "agent-browser",
      useCmd: false,
    };
  }

  return {
    command: "agent-browser.cmd",
    useCmd: true,
  };
}

const AGENT_BROWSER_INVOCATION = resolveAgentBrowserInvocation();
const AGENT_BROWSER_EXECUTABLE_PATH = resolveEdgeExecutablePath();

function quoteForCmd(arg) {
  const raw = String(arg);
  const escaped = raw.replace(/%/g, "^%");
  if (/^[A-Za-z0-9_./\\:=?@~()-]+$/.test(escaped)) {
    return escaped;
  }

  return `"${escaped.replace(/"/g, '""')}"`;
}

function runAgentBrowserCommand(commandArgs) {
  if (!AGENT_BROWSER_INVOCATION.useCmd) {
    return spawnSync(AGENT_BROWSER_INVOCATION.command, commandArgs, {
      timeout: AGENT_BROWSER_COMMAND_TIMEOUT_MS,
      maxBuffer: 10 * 1024 * 1024,
      env: process.env,
      windowsHide: true,
    });
  }

  const tempDir = mkdtempSync(join(tmpdir(), "loong-browser-companion-"));
  const stdoutPath = join(tempDir, "stdout.json");
  const stderrPath = join(tempDir, "stderr.txt");
  const commandLine = `${[AGENT_BROWSER_INVOCATION.command, ...commandArgs]
    .map(quoteForCmd)
    .join(" ")} > ${quoteForCmd(stdoutPath)} 2> ${quoteForCmd(stderrPath)}`;

  const result = spawnSync("cmd.exe", ["/d", "/c", commandLine], {
    timeout: AGENT_BROWSER_COMMAND_TIMEOUT_MS,
    windowsHide: true,
    stdio: "ignore",
  });

  const stdout = existsSync(stdoutPath) ? readFileSync(stdoutPath) : Buffer.alloc(0);
  const stderr = existsSync(stderrPath) ? readFileSync(stderrPath) : Buffer.alloc(0);
  try {
    rmSync(tempDir, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
  } catch {
    // Best-effort cleanup only. The redirected files are small and live in the OS temp directory.
  }

  return {
    ...result,
    stdout,
    stderr,
  };
}

function printVersion() {
  process.stdout.write(`loong-browser-companion ${COMPANION_VERSION}\n`);
}

function readStdinFrame() {
  return new Promise((resolve, reject) => {
    let input = "";
    process.stdin.setEncoding("utf8");
    const finish = (value) => {
      process.stdin.pause();
      process.stdin.removeListener("data", onData);
      process.stdin.removeListener("end", onEnd);
      process.stdin.removeListener("error", onError);
      resolve(value.replace(/\r$/, "").trim());
    };
    const onData = (chunk) => {
      input += chunk;
      const newlineIndex = input.indexOf("\n");
      if (newlineIndex >= 0) {
        finish(input.slice(0, newlineIndex));
      }
    };
    const onEnd = () => {
      finish(input);
    };
    const onError = (error) => {
      process.stdin.removeListener("data", onData);
      process.stdin.removeListener("end", onEnd);
      process.stdin.removeListener("error", onError);
      reject(error);
    };

    process.stdin.on("data", onData);
    process.stdin.on("end", onEnd);
    process.stdin.on("error", onError);
  });
}

function emitResponse(payload) {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function emitFailure(code, message) {
  emitResponse({
    ok: false,
    code,
    message,
  });
}

function runAgentBrowser(sessionId, args) {
  const commandArgs = ["--json", "--session", sessionId];
  if (AGENT_BROWSER_EXECUTABLE_PATH) {
    commandArgs.push("--executable-path", AGENT_BROWSER_EXECUTABLE_PATH);
  }
  commandArgs.push(...args);

  const result = runAgentBrowserCommand(commandArgs);

  if (result.error) {
    throw new Error(`agent-browser invocation failed: ${result.error.message}`);
  }

  if ((result.status ?? 1) !== 0) {
    const stderr = decodeCommandOutput(result.stderr).trim();
    const stdout = decodeCommandOutput(result.stdout).trim();
    throw new Error(
      `agent-browser exited with status ${result.status ?? 1}${stderr ? ` stderr=${stderr}` : ""}${stdout ? ` stdout=${stdout}` : ""}`,
    );
  }

  const raw = decodeCommandOutput(result.stdout).trim();
  if (!raw) {
    return null;
  }

  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`agent-browser returned invalid JSON: ${error.message}`);
  }

  if (!parsed?.success) {
    throw new Error(parsed?.error || "agent-browser reported failure");
  }

  return parsed.data ?? null;
}

function requireString(value, fieldName) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`missing required string field: ${fieldName}`);
  }

  return value.trim();
}

async function collectPageState(sessionId) {
  const [urlData, titleData] = await Promise.all([
    Promise.resolve(runAgentBrowser(sessionId, ["get", "url"])),
    Promise.resolve(runAgentBrowser(sessionId, ["get", "title"])),
  ]);

  return {
    page_url: typeof urlData?.url === "string" ? urlData.url : null,
    title: typeof titleData?.title === "string" ? titleData.title : null,
  };
}

async function runSnapshot(sessionId, mode) {
  const snapshotArgs = ["snapshot"];
  if (mode === "links") {
    snapshotArgs.push("-i", "--urls");
  } else {
    snapshotArgs.push("-i");
  }

  const [snapshotData, pageState] = await Promise.all([
    Promise.resolve(runAgentBrowser(sessionId, snapshotArgs)),
    collectPageState(sessionId),
  ]);

  if (mode === "html") {
    const htmlData = runAgentBrowser(sessionId, ["get", "html", "body"]);
    return {
      ...pageState,
      mode: "html",
      html: typeof htmlData?.html === "string" ? htmlData.html : "",
    };
  }

  return {
    ...pageState,
    mode: mode || "summary",
    snapshot: typeof snapshotData?.snapshot === "string" ? snapshotData.snapshot : "",
    refs:
      snapshotData && typeof snapshotData.refs === "object" && snapshotData.refs
        ? snapshotData.refs
        : {},
    origin:
      snapshotData && typeof snapshotData.origin === "string" ? snapshotData.origin : null,
  };
}

async function runWait(sessionId, args) {
  const condition =
    typeof args.condition === "string" ? args.condition.trim() : "";
  const timeoutMs =
    typeof args.timeout_ms === "number" && Number.isFinite(args.timeout_ms)
      ? Math.max(1, Math.floor(args.timeout_ms))
      : null;

  const waitArgs = ["wait"];
  if (!condition) {
    waitArgs.push(timeoutMs ? String(timeoutMs) : "500");
  } else if (condition.startsWith("load:")) {
    waitArgs.push("--load", condition.slice("load:".length) || "networkidle");
  } else if (condition.startsWith("url:")) {
    waitArgs.push("--url", condition.slice("url:".length));
  } else if (condition.startsWith("text:")) {
    waitArgs.push("--text", condition.slice("text:".length));
  } else if (condition.startsWith("fn:")) {
    waitArgs.push("--fn", condition.slice("fn:".length));
  } else if (/^\d+$/.test(condition)) {
    waitArgs.push(condition);
  } else {
    waitArgs.push(condition);
  }

  if (timeoutMs && !condition.startsWith("load:") && !/^\d+$/.test(condition)) {
    waitArgs.push("--timeout", String(timeoutMs));
  }

  const [waitData, pageState] = await Promise.all([
    Promise.resolve(runAgentBrowser(sessionId, waitArgs)),
    collectPageState(sessionId),
  ]);

  return {
    ...pageState,
    condition: condition || "500",
    wait: waitData,
  };
}

async function dispatchOperation(request) {
  const sessionId = requireString(request.session_id, "session_id");
  const args = request.arguments && typeof request.arguments === "object" ? request.arguments : {};

  switch (request.operation) {
    case "session.start": {
      const url = requireString(args.url, "arguments.url");
      const openData = runAgentBrowser(sessionId, ["open", url]);
      return {
        page_url: typeof openData?.url === "string" ? openData.url : url,
        title: typeof openData?.title === "string" ? openData.title : null,
      };
    }
    case "navigate": {
      const url = requireString(args.url, "arguments.url");
      const openData = runAgentBrowser(sessionId, ["open", url]);
      return {
        page_url: typeof openData?.url === "string" ? openData.url : url,
        title: typeof openData?.title === "string" ? openData.title : null,
      };
    }
    case "snapshot":
      return runSnapshot(sessionId, typeof args.mode === "string" ? args.mode.trim() : "summary");
    case "wait":
      return runWait(sessionId, args);
    case "click": {
      const selector = requireString(args.selector, "arguments.selector");
      runAgentBrowser(sessionId, ["click", selector]);
      const pageState = await collectPageState(sessionId);
      return {
        ...pageState,
        clicked: true,
        selector,
      };
    }
    case "type": {
      const selector = requireString(args.selector, "arguments.selector");
      const text = requireString(args.text, "arguments.text");
      runAgentBrowser(sessionId, ["type", selector, text]);
      const pageState = await collectPageState(sessionId);
      return {
        ...pageState,
        typed: true,
        selector,
        text,
      };
    }
    case "session.stop":
      runAgentBrowser(sessionId, ["close"]);
      return {
        closed: true,
      };
    default:
      throw new Error(`unsupported browser companion operation: ${request.operation}`);
  }
}

async function main() {
  if (process.argv.includes("--version")) {
    printVersion();
    return;
  }

  const input = await readStdinFrame();
  if (!input) {
    emitFailure("browser_companion_empty_request", "stdin did not contain a request");
    process.exitCode = 1;
    return;
  }

  let request;
  try {
    request = JSON.parse(input);
  } catch (error) {
    emitFailure(
      "browser_companion_invalid_request",
      `request JSON could not be parsed: ${error.message}`,
    );
    process.exitCode = 1;
    return;
  }

  try {
    const result = await dispatchOperation(request);
    emitResponse({
      ok: true,
      result,
    });
  } catch (error) {
    emitFailure(
      "browser_companion_operation_failed",
      error instanceof Error ? error.message : String(error),
    );
    process.exitCode = 1;
  }
}

void main();
