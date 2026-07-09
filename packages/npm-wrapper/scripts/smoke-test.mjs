import { ensureInstalledBinary } from "../lib/install.mjs";

await ensureInstalledBinary({
  allowDownload: false
}).catch(() => {
  process.exit(0);
});
