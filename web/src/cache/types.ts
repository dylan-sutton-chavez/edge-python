/* Common shape of `MemoryCache` and `IdbCache`, lets `fetch.ts`/`prefetch.ts`/`engine.ts` stay backend-agnostic. */
export interface CacheBackend {
    open(): void | Promise<void>
    getBytes(hash: string): Uint8Array | null | Promise<Uint8Array | null>
    putBytes(hash: string, bytes: Uint8Array): void | Promise<void>
    loadLockfile(): Map<string, string> | Promise<Map<string, string>>
    saveLockfile(entries: Iterable<[string, string]>): void | Promise<void>
    clear(): void | Promise<void>
    setVersion(version?: string | null): void | Promise<void>
    getVersion(): string | null | Promise<string | null>
}
