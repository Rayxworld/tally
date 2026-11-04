
export default async function handler(req, res) {
  if (req.method !== "GET") {
    return res.status(405).json({ error: "Method not allowed" });
  }

  const password = req.headers.authorization?.replace("Bearer ", "");

  if (!process.env.STATS_PASSWORD ||
      password !== process.env.STATS_PASSWORD) {
    return res.status(401).json({ error: "Unauthorized" });
  }

  const repo = process.env.GITHUB_REPO || "Rayxworld/tally";

  try {
    const response = await fetch(
      `https://api.github.com/repos/${repo}/releases?per_page=100`,
      {
        headers: {
          Accept: "application/vnd.github+json"
        }
      }
    );

    if (!response.ok) {
      throw new Error("Could not retrieve GitHub releases");
    }

    const releases = await response.json();

    const result = releases.map(release => ({
      version: release.tag_name,
      name: release.name,
      published_at: release.published_at,
      assets: release.assets.map(asset => ({
        name: asset.name,
        downloads: asset.download_count,
        size: asset.size
      })),
      total_downloads: release.assets.reduce(
        (sum, asset) => sum + asset.download_count, 0
      )
    }));

    const totalDownloads = result.reduce(
      (sum, release) => sum + release.total_downloads, 0
    );

    return res.status(200).json({
      total_downloads: totalDownloads,
      total_releases: result.length,
      releases: result
    });

  } catch (error) {
    return res.status(500).json({
      error: "Unable to load download statistics"
    });
  }
}
