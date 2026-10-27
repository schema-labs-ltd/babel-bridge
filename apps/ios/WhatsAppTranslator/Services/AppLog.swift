import OSLog

/// Unified logging for the iOS and macOS apps, one category per area, so device
/// issues can be read from Console or a sysdiagnose
/// (`log show --predicate 'subsystem == "com.vultuk.whatsapptranslator"'`).
///
/// Follow the server's rule: never log message text, captions, contact names or
/// phone numbers. Mark values `.public` only when they cannot carry any of
/// those, such as error descriptions, counts, states and HTTP status codes.
enum AppLog {
    static let subsystem = Bundle.main.bundleIdentifier ?? "com.vultuk.whatsapptranslator"

    /// Chat list, message history and background refresh.
    static let sync = Logger(subsystem: subsystem, category: "sync")
    /// The live websocket connection and WhatsApp link state.
    static let live = Logger(subsystem: subsystem, category: "live")
    /// Sending messages, media and reactions.
    static let send = Logger(subsystem: subsystem, category: "send")
    /// APNs registration and notification actions.
    static let push = Logger(subsystem: subsystem, category: "push")
    /// Media download and cache.
    static let media = Logger(subsystem: subsystem, category: "media")
    /// Errors shown to the user, logged with the alert title.
    static let ui = Logger(subsystem: subsystem, category: "ui")

    /// A content-free summary of an error, safe to log `.public`: its kind and
    /// code, never its message (server messages can echo request details).
    static func kind(of error: Error) -> String {
        switch error {
        case let error as URLError:
            return "URLError(\(error.code.rawValue))"
        case let error as APIError:
            switch error {
            case .notConfigured: return "APIError.notConfigured"
            case .invalidServer: return "APIError.invalidServer"
            case .unauthorized: return "APIError.unauthorized"
            case .server: return "APIError.server"
            case .decoding: return "APIError.decoding"
            }
        case is CancellationError:
            return "CancellationError"
        default:
            let error = error as NSError
            return "\(error.domain)(\(error.code))"
        }
    }
}
