// Read actual Mach-O deployment requirements; an install-name rewrite cannot
// make a library built for a newer OS compatible with an older one.
export function macosMinimumVersions(loadCommands) {
  let command;
  let platform;
  const versions = [];
  for (const line of loadCommands.split("\n")) {
    const text = line.trim();
    if (text.startsWith("cmd ")) {
      command = text.slice(4);
      platform = undefined;
    } else if (command === "LC_BUILD_VERSION") {
      if (text.startsWith("platform ")) platform = text.slice(9);
      const match = text.match(/^minos (\d+(?:\.\d+){0,2})$/);
      if (match && (platform === "1" || platform === "macos")) versions.push(match[1]);
    } else if (command === "LC_VERSION_MIN_MACOSX") {
      const match = text.match(/^version (\d+(?:\.\d+){0,2})$/);
      if (match) versions.push(match[1]);
    }
  }
  return versions;
}

function components(version) {
  if (!/^\d+(?:\.\d+){0,2}$/.test(version)) throw new Error(`Invalid macOS version: ${version}`);
  const parts = version.split(".").map(Number);
  return [parts[0], parts[1] || 0, parts[2] || 0];
}

export function assertMacosDeployment(loadCommands, minimum, file) {
  const allowed = components(minimum);
  const versions = macosMinimumVersions(loadCommands);
  if (!versions.length) throw new Error(`Cannot determine macOS deployment requirement: ${file}`);
  for (const version of versions) {
    const required = components(version);
    const difference = required.map((part, index) => part - allowed[index]).find((part) => part !== 0) || 0;
    if (difference > 0) {
      throw new Error(`${file} requires macOS ${version}, above bundle minimum ${minimum}. Build the application and FFmpeg dependencies on the macOS ${minimum} release runner, or use a matching SDK built for that minimum. Setting MACOSX_DEPLOYMENT_TARGET cannot lower a prebuilt library's requirement.`);
    }
  }
}
