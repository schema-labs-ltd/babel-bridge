import Foundation

/// Turns typed emoticons such as `:)` into emoji, as WhatsApp does. An emoticon converts when a space or
/// line break is typed after it, or when it ends a message being sent. It must stand on its own, so
/// `http://x`, `(:)`, `10:30` and `a<3` stay as typed.
/// Kept free of SwiftUI and UIKit so both unit-test targets can check it.
enum Emoticons {
    static let replacements: [String: String] = [
        ":)": "🙂", ":-)": "🙂", "=)": "🙂",
        ":(": "🙁", ":-(": "🙁",
        ":D": "😀", ":-D": "😀", "=D": "😀",
        "xD": "😆", "XD": "😆",
        ";)": "😉", ";-)": "😉",
        ":P": "😛", ":-P": "😛", ":p": "😛", ":-p": "😛",
        ";P": "😜", ";-P": "😜", ";p": "😜", ";-p": "😜",
        ":O": "😮", ":-O": "😮", ":o": "😮", ":-o": "😮",
        ":*": "😘", ":-*": "😘",
        ":|": "😐", ":-|": "😐",
        ":/": "😕", ":-/": "😕",
        // Smart punctuation turns the apostrophe into a curly one as it is typed.
        ":'(": "😢", ":’(": "😢",
        ">:(": "😠", ">:-(": "😠",
        ":$": "😳",
        "O:)": "😇", "O:-)": "😇", "0:)": "😇",
        "3:)": "😈", "3:-)": "😈",
        "^_^": "😊",
        "-_-": "😑",
        "<3": "❤️",
        "</3": "💔",
    ]

    /// What to write when the user types `typed` over the empty selection at `location`: when it is a
    /// single space or line break that completes an emoticon, the emoticon's UTF-16 range and the
    /// emoji followed by the typed character. The caret belongs after the replacement.
    static func replacement(forTyping typed: String, at location: Int, in text: String) -> (range: NSRange, text: String)? {
        guard typed.unicodeScalars.count == 1, let scalar = typed.unicodeScalars.first, scalar.properties.isWhitespace,
              let match = emoticon(endingAt: location, in: text) else { return nil }
        return (match.range, match.emoji + typed)
    }

    /// Converts an emoticon that ends the message, which never had a space typed after it. Only the
    /// last word changes, so an emoticon the user brought back with Undo earlier in the text is sent as typed.
    static func replacingTrailing(in text: String) -> String {
        guard let match = emoticon(endingAt: text.utf16.count, in: text),
              let range = Range(match.range, in: text) else { return text }
        return text.replacingCharacters(in: range, with: match.emoji)
    }

    /// The emoticon that ends exactly at the UTF-16 offset `location` and starts the text or follows whitespace.
    static func emoticon(endingAt location: Int, in text: String) -> (range: NSRange, emoji: String)? {
        let units = text.utf16
        guard location > 0, location <= units.count else { return nil }
        let end = units.index(units.startIndex, offsetBy: location)
        var start = end
        while start > units.startIndex {
            let previous = units.index(before: start)
            if isWhitespace(units[previous]) { break }
            start = previous
        }
        guard start < end, let word = String(units[start..<end]), let emoji = replacements[word] else { return nil }
        let offset = units.distance(from: units.startIndex, to: start)
        return (NSRange(location: offset, length: location - offset), emoji)
    }

    private static func isWhitespace(_ unit: UTF16.CodeUnit) -> Bool {
        // A surrogate half isn't a scalar on its own, and is never whitespace.
        Unicode.Scalar(unit)?.properties.isWhitespace ?? false
    }
}
