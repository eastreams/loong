const DEFAULT_TOKEN_PATH = "%USERPROFILE%\\.loong\\web-api-token";
const DEFAULT_TOKEN_ENV = "LOONGCLAW_WEB_TOKEN";

export function resolveTokenHintPath(tokenPath: string | null | undefined): string {
  const normalized = tokenPath?.trim();
  return normalized && normalized.length > 0 ? normalized : DEFAULT_TOKEN_PATH;
}

export function resolveTokenHintEnv(tokenEnv: string | null | undefined): string {
  const normalized = tokenEnv?.trim();
  return normalized && normalized.length > 0 ? normalized : DEFAULT_TOKEN_ENV;
}
