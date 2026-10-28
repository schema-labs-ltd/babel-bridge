import CoreGraphics
import CryptoKit
import Foundation

actor MediaCacheStore {
    static let shared = MediaCacheStore()

    private let directoryURL: URL
    private let maximumBytes: Int
    private let maximumRememberedSizes: Int
    private var rememberedSizes: [String: RememberedImageSize]?
    private var pendingSizeSave: Task<Void, Never>?

    init(directoryURL: URL? = nil, maximumBytes: Int = 512 * 1_024 * 1_024, maximumRememberedSizes: Int = 10_000) {
        self.directoryURL = directoryURL
            ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appending(path: "WhatsAppTranslator/MediaCache", directoryHint: .isDirectory)
        self.maximumBytes = maximumBytes
        self.maximumRememberedSizes = maximumRememberedSizes
    }

    /// Pixel sizes of photos and stickers that have been shown before. Older messages carry no
    /// dimensions from the server, so the chat reserves their frame from this instead of growing
    /// the bubble once the image arrives. It outlives evicted media files, which are much larger.
    func rememberedImageSizes() -> [String: CGSize] {
        loadRememberedSizesIfNeeded().mapValues { CGSize(width: $0.width, height: $0.height) }
    }

    func rememberImageSize(_ size: CGSize, messageID: String) {
        guard size.width > 0, size.height > 0 else { return }
        var sizes = loadRememberedSizesIfNeeded()
        let entry = RememberedImageSize(width: size.width, height: size.height, storedAt: Date())
        if let existing = sizes[messageID], existing.width == entry.width, existing.height == entry.height { return }
        sizes[messageID] = entry
        if sizes.count > maximumRememberedSizes {
            let overflow = sizes.count - maximumRememberedSizes
            for key in sizes.sorted(by: { $0.value.storedAt < $1.value.storedAt }).prefix(overflow).map(\.key) {
                sizes.removeValue(forKey: key)
            }
        }
        rememberedSizes = sizes
        scheduleSizeSave()
    }

    /// Writes any pending size changes now instead of after the save delay.
    func flushRememberedSizes() {
        pendingSizeSave?.cancel()
        pendingSizeSave = nil
        saveRememberedSizes()
    }

    func cachedURL(for messageID: String) throws -> URL? {
        guard FileManager.default.fileExists(atPath: directoryURL.path) else { return nil }
        let prefix = "\(cacheKey(for: messageID))."
        let files = try FileManager.default.contentsOfDirectory(
            at: directoryURL,
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles]
        )
        guard let url = files.first(where: { $0.lastPathComponent.hasPrefix(prefix) }) else {
            return nil
        }
        try? FileManager.default.setAttributes(
            [.modificationDate: Date()],
            ofItemAtPath: url.path
        )
        return url
    }

    @discardableResult
    func store(_ data: Data, messageID: String, fileExtension: String) throws -> URL {
        try FileManager.default.createDirectory(at: directoryURL, withIntermediateDirectories: true)
        try remove(messageID: messageID)

        let safeExtension = fileExtension
            .lowercased()
            .filter { $0.isLetter || $0.isNumber }
        let url = directoryURL.appending(
            path: "\(cacheKey(for: messageID)).\(safeExtension.isEmpty ? "bin" : safeExtension)",
            directoryHint: .notDirectory
        )
        try data.write(
            to: url,
            options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication]
        )
        var resourceValues = URLResourceValues()
        resourceValues.isExcludedFromBackup = true
        var mutableURL = url
        try? mutableURL.setResourceValues(resourceValues)
        try trimIfNeeded()
        return url
    }

    func remove(messageID: String) throws {
        guard FileManager.default.fileExists(atPath: directoryURL.path) else { return }
        let prefix = "\(cacheKey(for: messageID))."
        let files = try FileManager.default.contentsOfDirectory(
            at: directoryURL,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        )
        for url in files where url.lastPathComponent.hasPrefix(prefix) {
            try FileManager.default.removeItem(at: url)
        }
    }

    func clear() {
        try? FileManager.default.removeItem(at: directoryURL)
        pendingSizeSave?.cancel()
        pendingSizeSave = nil
        rememberedSizes = [:]
        try? FileManager.default.removeItem(at: sizesURL)
    }

    /// Kept beside the media folder so cache trimming never counts or evicts it.
    private var sizesURL: URL {
        directoryURL.deletingLastPathComponent()
            .appending(path: "\(directoryURL.lastPathComponent)-ImageSizes.json", directoryHint: .notDirectory)
    }

    private func loadRememberedSizesIfNeeded() -> [String: RememberedImageSize] {
        if let rememberedSizes { return rememberedSizes }
        let loaded = (try? Data(contentsOf: sizesURL))
            .flatMap { try? JSONDecoder().decode([String: RememberedImageSize].self, from: $0) } ?? [:]
        rememberedSizes = loaded
        return loaded
    }

    private func scheduleSizeSave() {
        guard pendingSizeSave == nil else { return }
        pendingSizeSave = Task {
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self.finishScheduledSizeSave()
        }
    }

    private func finishScheduledSizeSave() {
        pendingSizeSave = nil
        saveRememberedSizes()
    }

    private func saveRememberedSizes() {
        guard let rememberedSizes, let data = try? JSONEncoder().encode(rememberedSizes) else { return }
        try? FileManager.default.createDirectory(
            at: sizesURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try? data.write(to: sizesURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    private func cacheKey(for messageID: String) -> String {
        SHA256.hash(data: Data(messageID.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func trimIfNeeded() throws {
        let keys: Set<URLResourceKey> = [.fileSizeKey, .contentModificationDateKey, .isRegularFileKey]
        let files = try FileManager.default.contentsOfDirectory(
            at: directoryURL,
            includingPropertiesForKeys: Array(keys),
            options: [.skipsHiddenFiles]
        )
        let entries = try files.compactMap { url -> (url: URL, size: Int, date: Date)? in
            let values = try url.resourceValues(forKeys: keys)
            guard values.isRegularFile == true else { return nil }
            return (url, values.fileSize ?? 0, values.contentModificationDate ?? .distantPast)
        }
        var totalBytes = entries.reduce(0) { $0 + $1.size }
        let removableEntries = entries
            .sorted(by: { ($0.date, $0.url.path) < ($1.date, $1.url.path) })
            .dropLast()
        for entry in removableEntries where totalBytes > maximumBytes {
            try FileManager.default.removeItem(at: entry.url)
            totalBytes -= entry.size
        }
    }
}

private struct RememberedImageSize: Codable, Sendable {
    let width: Double
    let height: Double
    let storedAt: Date
}
