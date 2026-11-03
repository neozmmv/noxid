// Profile pictures as image URLs, loaded once per hash. A hash names one
// picture forever, so a URL never goes stale; a changed picture is a new hash.

import { api } from "./api";

const urls = $state<Record<string, string>>({});
const loading = new Set<string>();
const failed = new Set<string>();

/** The picture's URL, or null while it loads (or if there is none). Reading
 *  it in a template is enough: the URL appears when the bytes arrive. */
export function avatarUrl(hash: string | null | undefined): string | null {
  if (!hash) return null;
  const url = urls[hash];
  if (url) return url;
  if (!loading.has(hash) && !failed.has(hash)) {
    loading.add(hash);
    api
      .avatar(hash)
      .then((bytes) => (urls[hash] = URL.createObjectURL(new Blob([bytes]))))
      .catch(() => failed.add(hash))
      .finally(() => loading.delete(hash));
  }
  return null;
}
