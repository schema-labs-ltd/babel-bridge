import Foundation

struct ServerConfiguration: Codable, Equatable, Sendable {
    let baseURL: URL
    let password: String

    static func make(address: String, password: String) throws -> ServerConfiguration {
        var value = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { throw ConfigurationError.missingAddress }

        if !value.contains("://") {
            value = "https://\(value)"
        }

        guard var components = URLComponents(string: value),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              let host = components.host,
              !host.isEmpty else {
            throw ConfigurationError.invalidAddress
        }

        components.scheme = scheme
        components.query = nil
        components.fragment = nil
        components.percentEncodedPath = normalizedPercentEncodedPath(components.percentEncodedPath)
        guard let url = components.url else { throw ConfigurationError.invalidAddress }
        return ServerConfiguration(baseURL: url, password: password)
    }

    /// Joins an API path onto the server address and keeps any sub-path.
    ///
    /// Client paths start with `/`. `URL(string:relativeTo:)` treats that as an
    /// absolute path and would drop the prefix, so `https://example.com/prefix`
    /// would call `https://example.com/api/...`. The join uses a directory base
    /// (trailing slash) and a relative path so Foundation keeps the prefix and
    /// the request query encoding.
    func endpointURL(for path: String) -> URL? {
        guard let directory = directoryURL() else { return nil }
        let pieces = Self.requestPieces(path)
        var relative = Self.pathSegments(pieces.path).joined(separator: "/")
        if let query = pieces.query { relative += "?\(query)" }
        if let fragment = pieces.fragment { relative += "#\(fragment)" }
        guard !relative.isEmpty else { return directory.absoluteURL }
        return URL(string: relative, relativeTo: directory)?.absoluteURL
    }

    func webSocketURL(token: String?) -> URL? {
        guard let endpoint = endpointURL(for: "/ws"),
              var components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false) else {
            return nil
        }
        switch components.scheme?.lowercased() {
        case "https": components.scheme = "wss"
        case "http": components.scheme = "ws"
        default: return nil
        }
        if let token {
            components.queryItems = [URLQueryItem(name: "token", value: token)]
        }
        return components.url
    }

    private func directoryURL() -> URL? {
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
            return nil
        }
        let path = Self.normalizedPercentEncodedPath(components.percentEncodedPath)
        components.percentEncodedPath = path.isEmpty ? "/" : path + "/"
        components.query = nil
        components.fragment = nil
        return components.url
    }

    private static func normalizedPercentEncodedPath(_ percentEncodedPath: String) -> String {
        let segments = pathSegments(percentEncodedPath)
        guard !segments.isEmpty else { return "" }
        return "/" + segments.joined(separator: "/")
    }

    private static func pathSegments(_ percentEncodedPath: String) -> [String] {
        percentEncodedPath.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
    }

    private static func requestPieces(_ rawPath: String) -> (path: String, query: String?, fragment: String?) {
        let trimmed = rawPath.trimmingCharacters(in: .whitespacesAndNewlines)
        let fragmentParts = trimmed.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false)
        let beforeFragment = String(fragmentParts[0])
        let fragment = fragmentParts.count == 2 ? String(fragmentParts[1]) : nil
        let queryParts = beforeFragment.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)
        let path = String(queryParts[0])
        let query = queryParts.count == 2 ? String(queryParts[1]) : nil
        return (path, blankToNil(query), blankToNil(fragment))
    }

    private static func blankToNil(_ value: String?) -> String? {
        guard let value, !value.isEmpty else { return nil }
        return value
    }

    enum ConfigurationError: LocalizedError {
        case missingAddress
        case invalidAddress

        var errorDescription: String? {
            switch self {
            case .missingAddress: "Enter the web address of your translator."
            case .invalidAddress: "Enter a valid HTTP or HTTPS web address."
            }
        }
    }
}
