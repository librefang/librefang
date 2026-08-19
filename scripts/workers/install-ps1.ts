// Cloudflare Pages Function for install.ps1 redirect.
const RELEASE_URL = "https://api.github.com/repos/librefang/librefang/releases/latest";
const WINDOWS_ASSET = "librefang-x86_64-pc-windows-msvc.zip";

function releaseAssetUrl(data, assetName) {
  const asset = data.assets.find((candidate) =>
    candidate !== null &&
    typeof candidate === "object" &&
    typeof candidate.name === "string" &&
    candidate.name === assetName
  );
  if (!asset) return undefined;
  if (typeof asset.browser_download_url !== "string") return null;
  try {
    const url = new URL(asset.browser_download_url);
    const path = url.pathname.split("/");
    return url.origin === "https://github.com" &&
      path.length === 7 &&
      path[1] === "librefang" &&
      path[2] === "librefang" &&
      path[3] === "releases" &&
      path[4] === "download" &&
      path[5] !== "" &&
      path[6] === assetName &&
      url.search === "" &&
      url.hash === ""
      ? url.href
      : null;
  } catch {
    return null;
  }
}

export const onRequest = async () => {
  let response;
  try {
    response = await fetch(RELEASE_URL, {
      headers: {
        "Accept": "application/vnd.github+json",
        "User-Agent": "librefang-website"
      }
    });
  } catch {
    return new Response("Release service unavailable", { status: 502 });
  }
  if (!response.ok) {
    return new Response("Release service unavailable", { status: 502 });
  }

  let data;
  try {
    data = await response.json();
  } catch {
    return new Response("Invalid release service response", { status: 502 });
  }
  if (data === null || typeof data !== "object" || !Array.isArray(data.assets)) {
    return new Response("Invalid release service response", { status: 502 });
  }
  const assetUrl = releaseAssetUrl(data, WINDOWS_ASSET);
  if (typeof assetUrl === "string") {
    return Response.redirect(assetUrl, 302);
  }
  if (assetUrl === null) {
    return new Response("Invalid release service response", { status: 502 });
  }

  return new Response("No Windows x86_64 release found", { status: 404 });
};
