// `navigator.clipboard` is only defined in secure contexts (HTTPS or
// localhost). When the dashboard is served over plain HTTP on a LAN IP
// (`http://192.168.x.x:4545`) the clipboard API is `undefined`, so a bare
// `navigator.clipboard.writeText(...)` call throws with no feedback to the
// user. Fall back to the legacy `document.execCommand('copy')` path via a
// detached textarea so LAN-exposed dashboards still work.
export async function copyToClipboard(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // fall through to execCommand
    }
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-1000px";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
