const PULL_REQUEST_FILE_LIMIT = 3000;
const PUSH_COMPARE_FILE_LIMIT = 300;

const CI_RUST_PATTERNS = [
  /^\.cargo\//,
  /^Cargo\.toml$/,
  /^Cargo\.lock$/,
  /^Cross\.toml$/,
  /^rust-toolchain\.toml$/,
  /^clippy\.toml$/,
  /^rustfmt\.toml$/,
  /^Taskfile\.yml$/,
  /^crates\//,
  /^examples\//,
  /^patches\//,
  /^\.github\/workflows\/(ci|release)\.yml$/,
];

const CI_DOCS_SITE_PATTERNS = [
  /^site\//,
  /^\.github\/workflows\/ci\.yml$/,
];

const SECURITY_PATTERNS = [
  /^\.cargo\/config\.toml$/,
  /^\.github\/dependabot\.yml$/,
  /^\.github\/workflows\/security\.yml$/,
  /^Cargo\.toml$/,
  /^Cargo\.lock$/,
  /^deny\.toml$/,
  /^patches\//,
  /(^|\/)Cargo\.toml$/,
];

const CODEQL_PATTERNS = [
  /^\.cargo\/config\.toml$/,
  /^\.github\/workflows\/codeql\.yml$/,
  /^Cargo\.toml$/,
  /^Cargo\.lock$/,
  /^crates\//,
  /^examples\//,
  /^patches\//,
];

function isNonEmptyString(value) {
  const isString = typeof value === "string";
  const hasLength = isString && value.length > 0;
  return hasLength;
}

function normalizeChangedFile(file) {
  const normalizedFile = {};

  if (isNonEmptyString(file)) {
    normalizedFile.filename = file;
    return normalizedFile;
  }

  const currentPath = file?.filename;
  const previousPath = file?.previous_filename;

  if (isNonEmptyString(currentPath)) {
    normalizedFile.filename = currentPath;
  }

  if (isNonEmptyString(previousPath)) {
    normalizedFile.previous_filename = previousPath;
  }

  return normalizedFile;
}

function isTruncatedPullRequestFileList(eventName, files) {
  const isPullRequest = eventName === "pull_request";
  const fileCount = files.length;
  const mayBeTruncated = isPullRequest && fileCount >= PULL_REQUEST_FILE_LIMIT;
  return mayBeTruncated;
}

function isTruncatedPushCompareFileList(eventName, files) {
  const isPush = eventName === "push";
  const fileCount = files.length;
  const mayBeTruncated = isPush && fileCount >= PUSH_COMPARE_FILE_LIMIT;
  return mayBeTruncated;
}

function buildOwnerRepo(context) {
  const owner = context.repo.owner;
  const repo = context.repo.repo;
  return { owner, repo };
}

function createForcedRunContext(reason) {
  const changedFileContext = {};
  changedFileContext.forceRun = true;
  changedFileContext.reason = reason;
  changedFileContext.files = [];
  return changedFileContext;
}

function createCollectedFileContext(files) {
  const changedFileContext = {};
  changedFileContext.forceRun = false;
  changedFileContext.reason = "";
  changedFileContext.files = files;
  return changedFileContext;
}

function addPathIfPresent(pathSet, pathValue) {
  if (!isNonEmptyString(pathValue)) {
    return;
  }

  pathSet.add(pathValue);
}

export function collectTouchedPaths(files) {
  const touchedPathSet = new Set();

  for (const file of files) {
    const normalizedFile = normalizeChangedFile(file);
    const currentPath = normalizedFile.filename;
    const previousPath = normalizedFile.previous_filename;

    addPathIfPresent(touchedPathSet, currentPath);
    addPathIfPresent(touchedPathSet, previousPath);
  }

  const touchedPaths = Array.from(touchedPathSet.values());
  return touchedPaths;
}

function matchesAnyPattern(paths, patterns) {
  for (const pathValue of paths) {
    for (const pattern of patterns) {
      const doesMatch = pattern.test(pathValue);
      if (doesMatch) {
        return true;
      }
    }
  }

  return false;
}

async function listPullRequestFiles(runtime) {
  const ownerRepo = buildOwnerRepo(runtime.context);
  const pullRequestNumber = runtime.context.payload.pull_request.number;
  const listOptions = {};
  listOptions.owner = ownerRepo.owner;
  listOptions.repo = ownerRepo.repo;
  listOptions.pull_number = pullRequestNumber;
  listOptions.per_page = 100;

  const files = await runtime.github.paginate(
    runtime.github.rest.pulls.listFiles,
    listOptions,
  );

  return files;
}

async function listPushCompareFiles(runtime) {
  const ownerRepo = buildOwnerRepo(runtime.context);
  const baseSha = runtime.context.payload.before;
  const headSha = runtime.context.sha;
  const compareOptions = {};
  compareOptions.owner = ownerRepo.owner;
  compareOptions.repo = ownerRepo.repo;
  compareOptions.base = baseSha;
  compareOptions.head = headSha;
  compareOptions.per_page = 100;

  const comparison = await runtime.github.rest.repos.compareCommits(compareOptions);
  const files = comparison.data.files || [];
  return files;
}

export async function collectChangedFileContext(runtime, options) {
  const workflowLabel = options.workflowLabel;
  const forceRunEvents = new Set(options.forceRunEvents || []);
  const eventName = runtime.context.eventName;
  const forceRunMessage = `${workflowLabel}: defaulting to full validation.`;

  if (forceRunEvents.has(eventName)) {
    const reason = `${workflowLabel}: ${eventName} requires full validation.`;
    runtime.core.info(reason);
    return createForcedRunContext(reason);
  }

  try {
    if (eventName === "pull_request") {
      const files = await listPullRequestFiles(runtime);
      const isTruncated = isTruncatedPullRequestFileList(eventName, files);

      if (isTruncated) {
        const fileCount = files.length;
        const reason =
          `${workflowLabel}: pull request files API returned ${fileCount} files, ` +
          "which may be truncated; defaulting to full validation.";
        runtime.core.warning(reason);
        return createForcedRunContext(reason);
      }

      return createCollectedFileContext(files);
    }

    if (eventName === "push") {
      const baseSha = runtime.context.payload.before;
      const hasBaseSha = isNonEmptyString(baseSha);
      const isSyntheticBaseSha = hasBaseSha && /^0+$/.test(baseSha);

      if (!hasBaseSha || isSyntheticBaseSha) {
        const reason =
          `${workflowLabel}: push before SHA is unavailable; defaulting to full validation.`;
        runtime.core.info(reason);
        return createForcedRunContext(reason);
      }

      const files = await listPushCompareFiles(runtime);
      const isTruncated = isTruncatedPushCompareFileList(eventName, files);

      if (isTruncated) {
        const fileCount = files.length;
        const reason =
          `${workflowLabel}: compare API returned ${fileCount} files, ` +
          "which may be truncated; defaulting to full validation.";
        runtime.core.warning(reason);
        return createForcedRunContext(reason);
      }

      return createCollectedFileContext(files);
    }
  } catch (error) {
    const errorMessage = `${workflowLabel}: failed to classify changed files: ${error.message}`;
    runtime.core.warning(errorMessage);
    return createForcedRunContext(errorMessage);
  }

  const unsupportedReason = `${workflowLabel}: unsupported event '${eventName}'; ${forceRunMessage}`;
  runtime.core.info(unsupportedReason);
  return createForcedRunContext(unsupportedReason);
}

export function classifyCiJobSet(changedFileContext) {
  const shouldForceRun = changedFileContext.forceRun;
  const touchedPaths = collectTouchedPaths(changedFileContext.files);
  const touchedPathsJson = JSON.stringify(touchedPaths);

  if (shouldForceRun) {
    const forcedOutputs = {};
    forcedOutputs.runRustJobs = true;
    forcedOutputs.runDocsSite = true;
    forcedOutputs.touchedPaths = touchedPaths;
    forcedOutputs.touchedPathsJson = touchedPathsJson;
    return forcedOutputs;
  }

  const runRustJobs = matchesAnyPattern(touchedPaths, CI_RUST_PATTERNS);
  const runDocsSite = matchesAnyPattern(touchedPaths, CI_DOCS_SITE_PATTERNS);
  const routeOutputs = {};
  routeOutputs.runRustJobs = runRustJobs;
  routeOutputs.runDocsSite = runDocsSite;
  routeOutputs.touchedPaths = touchedPaths;
  routeOutputs.touchedPathsJson = touchedPathsJson;
  return routeOutputs;
}

export function classifySecurityJobSet(changedFileContext) {
  const shouldForceRun = changedFileContext.forceRun;
  const touchedPaths = collectTouchedPaths(changedFileContext.files);

  if (shouldForceRun) {
    const forcedOutputs = {};
    forcedOutputs.runAdvisoryChecks = true;
    forcedOutputs.touchedPaths = touchedPaths;
    return forcedOutputs;
  }

  const runAdvisoryChecks = matchesAnyPattern(touchedPaths, SECURITY_PATTERNS);
  const routeOutputs = {};
  routeOutputs.runAdvisoryChecks = runAdvisoryChecks;
  routeOutputs.touchedPaths = touchedPaths;
  return routeOutputs;
}

export function classifyCodeqlJobSet(changedFileContext) {
  const shouldForceRun = changedFileContext.forceRun;
  const touchedPaths = collectTouchedPaths(changedFileContext.files);

  if (shouldForceRun) {
    const forcedOutputs = {};
    forcedOutputs.runAnalysis = true;
    forcedOutputs.touchedPaths = touchedPaths;
    return forcedOutputs;
  }

  const runAnalysis = matchesAnyPattern(touchedPaths, CODEQL_PATTERNS);
  const routeOutputs = {};
  routeOutputs.runAnalysis = runAnalysis;
  routeOutputs.touchedPaths = touchedPaths;
  return routeOutputs;
}

export function writeRouteOutputs(core, routeOutputs) {
  const outputEntries = Object.entries(routeOutputs);

  for (const [outputName, outputValue] of outputEntries) {
    if (outputName === "touchedPaths") {
      continue;
    }

    const normalizedValue =
      typeof outputValue === "boolean" ? (outputValue ? "true" : "false") : outputValue;
    core.setOutput(outputName, normalizedValue);
  }
}

export function describeTouchedPaths(routeOutputs) {
  const touchedPaths = routeOutputs.touchedPaths || [];
  const touchedPathSummary = touchedPaths.join(", ");
  return touchedPathSummary;
}
