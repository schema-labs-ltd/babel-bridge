import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

final class ServerURLTests: XCTestCase {
    func testEndpointURLsKeepTheServerSubpath() throws {
        let rows: [(address: String, path: String, expected: String)] = [
            ("https://example.com", "/api/health", "https://example.com/api/health"),
            ("https://example.com/", "/api/health", "https://example.com/api/health"),
            ("https://example.com/prefix", "/api/health", "https://example.com/prefix/api/health"),
            ("https://example.com/prefix/", "/api/health", "https://example.com/prefix/api/health"),
            ("https://example.com/prefix/", "/api/messages", "https://example.com/prefix/api/messages"),
            ("example.com/prefix", "/api/status", "https://example.com/prefix/api/status"),
            ("  example.com/prefix/  ", "/api/qr", "https://example.com/prefix/api/qr"),
            ("http://example.com/prefix", "/api/status", "http://example.com/prefix/api/status"),
            ("HTTP://example.com/prefix/", "/api/contacts", "http://example.com/prefix/api/contacts"),
            ("https://example.com:8443/prefix", "/api/send", "https://example.com:8443/prefix/api/send"),
            ("http://example.com:8080/prefix/", "/api/send-image", "http://example.com:8080/prefix/api/send-image"),
            ("example.com:8443/prefix", "/api/health", "https://example.com:8443/prefix/api/health"),
            ("http://127.0.0.1:8787/bridge/", "/api/health", "http://127.0.0.1:8787/bridge/api/health"),
            ("https://example.com/prefix/v1/", "/api/contacts", "https://example.com/prefix/v1/api/contacts"),
            ("https://example.com//prefix///", "/api//health", "https://example.com/prefix/api/health"),
            ("https://example.com/prefix?discard=1", "/api/feed?limit=50", "https://example.com/prefix/api/feed?limit=50"),
            ("https://example.com/prefix/?discard=1#section", "/api/health", "https://example.com/prefix/api/health"),
            ("https://example.com/prefix#section", "/api/health#note", "https://example.com/prefix/api/health#note"),
            ("https://example.com/Prefix/", "/api/health", "https://example.com/Prefix/api/health"),
            (
                "https://example.com/prefix",
                "/api/messages/family%2Fparents@g.us?limit=50&before_id=a%2Fb",
                "https://example.com/prefix/api/messages/family%2Fparents@g.us?limit=50&before_id=a%2Fb"
            ),
            (
                "https://example.com/prefix/",
                "/api/photo-albums/album-1/images/0",
                "https://example.com/prefix/api/photo-albums/album-1/images/0"
            ),
            (
                "https://example.com/prefix",
                "/api/photo-albums/album-1/send",
                "https://example.com/prefix/api/photo-albums/album-1/send"
            ),
            (
                "https://example.com/my%20bridge/",
                "/api/link-preview?url=https%3A%2F%2Fexample.com%2Fa.jpg",
                "https://example.com/my%20bridge/api/link-preview?url=https%3A%2F%2Fexample.com%2Fa.jpg"
            ),
            ("https://example.com", "/api/health/", "https://example.com/api/health"),
            (" translator.example.com/ ", "/api/health", "https://translator.example.com/api/health"),
        ]

        for row in rows {
            let configuration = try ServerConfiguration.make(address: row.address, password: "test")
            let url = try XCTUnwrap(configuration.endpointURL(for: row.path), row.address)
            XCTAssertEqual(url.absoluteString, row.expected, "\(row.address) + \(row.path)")
            XCTAssertFalse(url.path.contains("//"), row.expected)
        }
    }

    func testWebSocketURLsKeepTheServerSubpath() throws {
        let rows: [(address: String, token: String?, expected: String)] = [
            ("https://example.com", nil, "wss://example.com/ws"),
            ("https://example.com/", nil, "wss://example.com/ws"),
            ("https://example.com/prefix", nil, "wss://example.com/prefix/ws"),
            ("https://example.com/prefix/", nil, "wss://example.com/prefix/ws"),
            ("http://example.com/prefix", nil, "ws://example.com/prefix/ws"),
            ("http://example.com:8080/prefix/", nil, "ws://example.com:8080/prefix/ws"),
            ("https://example.com:8443/prefix/", "fixture", "wss://example.com:8443/prefix/ws?token=fixture"),
            ("example.com/prefix", nil, "wss://example.com/prefix/ws"),
            ("http://127.0.0.1:8787/bridge", nil, "ws://127.0.0.1:8787/bridge/ws"),
        ]

        for row in rows {
            let configuration = try ServerConfiguration.make(address: row.address, password: "test")
            let url = try XCTUnwrap(configuration.webSocketURL(token: row.token), row.address)
            XCTAssertEqual(url.absoluteString, row.expected, row.address)
            XCTAssertFalse(url.path.contains("//"), row.expected)
        }
    }

    func testServerAddressNormalizesEquivalentSpellings() throws {
        let root = try ServerConfiguration.make(address: " translator.example.com/ ", password: "test")
        XCTAssertEqual(root.baseURL.absoluteString, "https://translator.example.com")

        let prefixed = try ServerConfiguration.make(address: "https://example.com/prefix", password: "test")
        let slashed = try ServerConfiguration.make(address: "https://example.com/prefix/", password: "test")
        let queried = try ServerConfiguration.make(address: "HTTPS://example.com/prefix?discard=1#section", password: "test")
        XCTAssertEqual(prefixed.baseURL.absoluteString, "https://example.com/prefix")
        XCTAssertEqual(prefixed.baseURL, slashed.baseURL)
        XCTAssertEqual(prefixed.baseURL, queried.baseURL)

        let reloaded = try ServerConfiguration.make(address: prefixed.baseURL.absoluteString, password: "test")
        XCTAssertEqual(reloaded.baseURL, prefixed.baseURL)
        XCTAssertEqual(
            reloaded.endpointURL(for: "/api/health")?.absoluteString,
            "https://example.com/prefix/api/health"
        )
    }

    func testInvalidServerAddressReportsAClearError() {
        XCTAssertThrowsError(try ServerConfiguration.make(address: "   ", password: "test")) { error in
            XCTAssertEqual(error.localizedDescription, "Enter the web address of your translator.")
        }

        for address in ["ftp://example.com", "https://", "http://", "not a url", "/prefix", "://example.com"] {
            XCTAssertThrowsError(try ServerConfiguration.make(address: address, password: "test")) { error in
                XCTAssertEqual(error.localizedDescription, "Enter a valid HTTP or HTTPS web address.", address)
            }
        }
    }
}
