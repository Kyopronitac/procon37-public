// Imported histories, including records saved by older versions, never supply
// executable commands to the rerun dialog.
export function rerunConfig(manifest, localConfig) {
  const config = structuredClone(manifest.config);
  if (manifest.importedFrom) {
    config.build = structuredClone(localConfig.build);
    config.solver = structuredClone(localConfig.solver);
    config.solverParallel = localConfig.solverParallel;
    if (config.opponents) config.opponents = structuredClone(localConfig.opponents);
  }
  return config;
}
