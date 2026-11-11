// Which About tab to open next, for links from elsewhere (the What's new
// banner opens Release notes). Read once by About when it opens.
let pending: string | null = null;

export function requestAboutTab(id: string) {
  pending = id;
}

export function takeAboutTab(): string | null {
  const id = pending;
  pending = null;
  return id;
}
