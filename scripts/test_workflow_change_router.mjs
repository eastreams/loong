import assert from "node:assert/strict";

import {
  classifyCiJobSet,
  classifyCodeqlJobSet,
  classifySecurityJobSet,
  collectChangedFileContext,
  collectTouchedPaths,
  writeRouteOutputs,
} from "./workflow_change_router.mjs";

function createRuntime(eventName, files, beforeSha = "abc123", headSha = "def456") {
  const messages = [];
  const runtime = {};
  runtime.context = {
    eventName,
    payload: {
      before: beforeSha,
      pull_request: {
        number: 42,
      },
    },
    repo: {
      owner: "loong-ai",
      repo: "loong",
    },
    sha: headSha,
  };
  runtime.github = {
    paginate: async () => files,
    rest: {
      pulls: {
        listFiles: {},
      },
      repos: {
        compareCommits: async () => {
          return {
            data: {
              files,
            },
          };
        },
      },
    },
  };
  runtime.core = {
    info: (message) => {
      messages.push(`info:${message}`);
    },
    warning: (message) => {
      messages.push(`warning:${message}`);
    },
  };
  runtime.messages = messages;
  return runtime;
}

async function runTests() {
  const renamedRustFile = {};
  renamedRustFile.filename = "docs/runtime.md";
  renamedRustFile.previous_filename = "crates/app/src/runtime.rs";

  const renamedDenyFile = {};
  renamedDenyFile.filename = "docs/deny.md";
  renamedDenyFile.previous_filename = "deny.toml";

  const renamedCodeFile = {};
  renamedCodeFile.filename = "docs/moved.md";
  renamedCodeFile.previous_filename = "crates/app/src/channel/mod.rs";

  const renamedSiteFile = {};
  renamedSiteFile.filename = "README.md";
  renamedSiteFile.previous_filename = "site/index.mdx";

  const touchedPaths = collectTouchedPaths([
    renamedRustFile,
    renamedDenyFile,
    renamedCodeFile,
    renamedSiteFile,
  ]);

  assert.deepEqual(touchedPaths, [
    "docs/runtime.md",
    "crates/app/src/runtime.rs",
    "docs/deny.md",
    "deny.toml",
    "docs/moved.md",
    "crates/app/src/channel/mod.rs",
    "README.md",
    "site/index.mdx",
  ]);

  const ciRenameContext = {};
  ciRenameContext.forceRun = false;
  ciRenameContext.files = [renamedRustFile, renamedSiteFile];
  const ciRenameOutputs = classifyCiJobSet(ciRenameContext);
  assert.equal(ciRenameOutputs.runRustJobs, true);
  assert.equal(ciRenameOutputs.runDocsSite, true);
  assert.equal(
    ciRenameOutputs.touchedPathsJson,
    JSON.stringify([
      "docs/runtime.md",
      "crates/app/src/runtime.rs",
      "README.md",
      "site/index.mdx",
    ]),
  );

  const emittedOutputs = {};
  const outputCore = {};
  outputCore.setOutput = (name, value) => {
    emittedOutputs[name] = value;
  };
  writeRouteOutputs(outputCore, {
    runRustJobs: true,
    runDocsSite: false,
    touchedPaths: ["docs/runtime.md"],
    touchedPathsJson: JSON.stringify(["docs/runtime.md"]),
    runAdvisoryChecks: true,
    runAnalysis: false,
  });
  assert.deepEqual(emittedOutputs, {
    run_rust_jobs: "true",
    run_docs_site: "false",
    touched_paths_json: JSON.stringify(["docs/runtime.md"]),
    run_advisory_checks: "true",
    run_analysis: "false",
  });

  const securityCargoConfigContext = {};
  securityCargoConfigContext.forceRun = false;
  securityCargoConfigContext.files = [{ filename: ".cargo/config.toml" }];
  const securityCargoConfigOutputs = classifySecurityJobSet(securityCargoConfigContext);
  assert.equal(securityCargoConfigOutputs.runAdvisoryChecks, true);

  const securityRenameContext = {};
  securityRenameContext.forceRun = false;
  securityRenameContext.files = [renamedDenyFile];
  const securityRenameOutputs = classifySecurityJobSet(securityRenameContext);
  assert.equal(securityRenameOutputs.runAdvisoryChecks, true);

  const codeqlRenameContext = {};
  codeqlRenameContext.forceRun = false;
  codeqlRenameContext.files = [renamedCodeFile];
  const codeqlRenameOutputs = classifyCodeqlJobSet(codeqlRenameContext);
  assert.equal(codeqlRenameOutputs.runAnalysis, true);

  const ciMergeGroupRuntime = createRuntime("merge_group", []);
  const ciMergeGroupContext = await collectChangedFileContext(ciMergeGroupRuntime, {
    workflowLabel: "CI",
    forceRunEvents: ["merge_group"],
  });
  assert.equal(ciMergeGroupContext.forceRun, true);

  const securityScheduleRuntime = createRuntime("schedule", []);
  const securityScheduleContext = await collectChangedFileContext(
    securityScheduleRuntime,
    {
      workflowLabel: "Security",
      forceRunEvents: ["schedule", "workflow_dispatch", "merge_group"],
    },
  );
  assert.equal(securityScheduleContext.forceRun, true);

  const pushFiles = [];
  for (let index = 0; index < 300; index += 1) {
    pushFiles.push({ filename: `crates/app/src/file-${index}.rs` });
  }

  const pushRuntime = createRuntime("push", pushFiles);
  const pushContext = await collectChangedFileContext(pushRuntime, {
    workflowLabel: "CI",
    forceRunEvents: ["merge_group"],
  });
  assert.equal(pushContext.forceRun, true);

  const pullRequestFiles = [];
  for (let index = 0; index < 3000; index += 1) {
    pullRequestFiles.push({ filename: `crates/app/src/file-${index}.rs` });
  }

  const pullRequestRuntime = createRuntime("pull_request", pullRequestFiles);
  const pullRequestContext = await collectChangedFileContext(
    pullRequestRuntime,
    {
      workflowLabel: "CodeQL",
      forceRunEvents: ["schedule", "merge_group"],
    },
  );
  assert.equal(pullRequestContext.forceRun, true);

  const ordinaryPullRequestRuntime = createRuntime("pull_request", [
    { filename: "docs/README.md" },
  ]);
  const ordinaryPullRequestContext = await collectChangedFileContext(
    ordinaryPullRequestRuntime,
    {
      workflowLabel: "Security",
      forceRunEvents: ["schedule", "workflow_dispatch", "merge_group"],
    },
  );
  assert.equal(ordinaryPullRequestContext.forceRun, false);

  console.log("workflow change router checks passed");
}

await runTests();
