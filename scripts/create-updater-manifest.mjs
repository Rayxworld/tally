import { readFile, readdir, writeFile } from 'node:fs/promises'
import { basename, join } from 'node:path'

const [tag, bundleDirectory, outputPath = 'latest.json'] = process.argv.slice(2)
const version = tag?.replace(/^v/, '')

if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version) || !bundleDirectory) {
  throw new Error('Usage: node scripts/create-updater-manifest.mjs <version-tag> <nsis-bundle-directory> [output-file]')
}

const assets = await readdir(bundleDirectory)
const installerName = assets.find(name => name.endsWith(`_${version}_x64-setup.exe`))
if (!installerName) {
  throw new Error(`No x64 NSIS installer for version ${version} found in ${bundleDirectory}`)
}

const signature = (await readFile(join(bundleDirectory, `${installerName}.sig`), 'utf8')).trim()
if (!signature) {
  throw new Error(`The installer signature is empty: ${installerName}.sig`)
}

const notes = (await readFile('RELEASE_NOTES.md', 'utf8')).trim()
const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64-nsis': {
      signature,
      url: `https://github.com/Rayxworld/tally/releases/download/v${version}/${encodeURIComponent(basename(installerName))}`,
    },
  },
}

await writeFile(outputPath, `${JSON.stringify(manifest, null, 2)}\n`)
