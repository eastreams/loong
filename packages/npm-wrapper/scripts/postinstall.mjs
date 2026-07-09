import process from "node:process";

import { ensureInstalledBinary } from "../lib/install.mjs";

const skipInstall = process.env.LOONG_NPM_SKIP_DOWNLOAD === "1";

if (!skipInstall) {
  await ensureInstalledBinary({
    allowDownload: true
  });
}
