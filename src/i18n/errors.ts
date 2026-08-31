// Commands fail with a stable code (see `src-tauri/src/commands.rs`), not with
// a sentence. Turning the code into words is the frontend's job, because the
// words depend on a language the backend does not know.
import { i18n } from "./index";

/**
 * A code with no catalogue entry (an older build talking to a newer backend)
 * becomes the generic message; the raw code is not for the user to read.
 */
export function errorMessage(code: unknown): string {
  const key = `errors.${String(code)}`;
  const { t, te } = i18n.global;
  return te(key) ? t(key) : t("errors.unknown");
}
