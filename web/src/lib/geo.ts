export function haversine(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const R = 6371000;
  const dLat = ((lat2 - lat1) * Math.PI) / 180;
  const dLon = ((lon2 - lon1) * Math.PI) / 180;
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.cos((lat1 * Math.PI) / 180) *
      Math.cos((lat2 * Math.PI) / 180) *
      Math.sin(dLon / 2) ** 2;
  return R * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
}

export function estimateWorldSize(
  bbox: [number, number, number, number],
  scale: number,
  surfaceThickness = 4,
): { widthBlocks: number; depthBlocks: number; chunks: number; fileSizeBytes: number } {
  const [minLon, minLat, maxLon, maxLat] = bbox;
  const centerLat = (minLat + maxLat) / 2;
  const widthM = haversine(centerLat, minLon, centerLat, maxLon);
  const depthM = haversine(minLat, minLon, maxLat, minLon);
  const widthBlocks = Math.round(widthM / scale);
  const depthBlocks = Math.round(depthM / scale);
  const chunks = Math.ceil(widthBlocks / 16) * Math.ceil(depthBlocks / 16);
  // Empirical base: ~400 bytes/chunk with full underground fill (129 blocks).
  // Scale by the ratio of filled subchunks (16 blocks each), with a floor of
  // ~80 bytes/chunk for chunk overhead even when mostly empty.
  const fullSubchunks = Math.ceil(129 / 16); // 9
  const activeSubchunks = Math.max(1, Math.ceil(surfaceThickness / 16));
  const bytesPerChunk = Math.round(80 + 320 * (activeSubchunks / fullSubchunks));
  const fileSizeBytes = Math.round(chunks * bytesPerChunk);
  return { widthBlocks, depthBlocks, chunks, fileSizeBytes };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
