export function isLazyChunkLoadError(error: Error) {
  return (
    error.name === "ChunkLoadError" ||
    /Failed to fetch dynamically imported module|Importing a module script failed|Loading (?:CSS )?chunk [^ ]+ failed/i.test(
      error.message
    )
  )
}
