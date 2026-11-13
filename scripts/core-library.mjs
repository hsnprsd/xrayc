export function coreLibraryName(platform = process.platform) {
  const names = { linux: 'libxrayc-core.so', darwin: 'libxrayc-core.dylib', win32: 'xrayc-core.dll' };
  if (!names[platform]) throw new Error(`Unsupported core platform: ${platform}`);
  return names[platform];
}
