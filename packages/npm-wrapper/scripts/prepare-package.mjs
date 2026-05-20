import { chmodSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const binPath = join(root, "bin", "loong.js");

if (existsSync(binPath)) {
  chmodSync(binPath, 0o755);
}
