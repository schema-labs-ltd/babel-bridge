import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

final class EmoticonsTests: XCTestCase {
    func testTypingASpaceAfterAnEmoticonReplacesIt() throws {
        let edit = try XCTUnwrap(Emoticons.replacement(forTyping: " ", at: 8, in: "hello :)"))
        XCTAssertEqual(edit.range, NSRange(location: 6, length: 2))
        XCTAssertEqual(edit.text, "🙂 ")

        let newline = try XCTUnwrap(Emoticons.replacement(forTyping: "\n", at: 3, in: ":-D"))
        XCTAssertEqual(newline.range, NSRange(location: 0, length: 3))
        XCTAssertEqual(newline.text, "😀\n")
    }

    func testTypingMidTextCountsUTF16Units() throws {
        // The flag is two surrogate pairs, so the emoticon starts at offset 5, not 3.
        let edit = try XCTUnwrap(Emoticons.replacement(forTyping: " ", at: 7, in: "🇬🇧 <3later"))
        XCTAssertEqual(edit.range, NSRange(location: 5, length: 2))
        XCTAssertEqual(edit.text, "❤️ ")
    }

    func testEveryListedEmoticonConverts() {
        for (emoticon, emoji) in Emoticons.replacements {
            XCTAssertEqual(Emoticons.replacement(forTyping: " ", at: emoticon.utf16.count, in: emoticon)?.text, emoji + " ", emoticon)
        }
    }

    func testTypingLeavesOtherTextAlone() {
        let rows: [(typed: String, location: Int, text: String)] = [
            ("a", 2, ":)"), // not a space
            ("  ", 2, ":)"), // more than one character, such as a paste
            (" ", 1, ":)"), // the space splits the emoticon
            (" ", 7, "smile:)"), // inside a word
            (" ", 18, "http://example.com"),
            (" ", 3, "(:)"),
            (" ", 5, "10:30"),
            (" ", 2, "🙂"), // already an emoji
            (" ", 0, ""),
            (" ", 9, ":)"), // past the end
        ]
        for row in rows {
            XCTAssertNil(Emoticons.replacement(forTyping: row.typed, at: row.location, in: row.text), "\(row)")
        }
    }

    func testSendingConvertsOnlyAnEmoticonThatEndsTheMessage() {
        let rows: [(draft: String, sent: String)] = [
            (":)", "🙂"),
            ("see you <3", "see you ❤️"),
            ("ok\n:P", "ok\n😛"),
            // An emoticon restored with Undo, or followed by a space, is sent as typed.
            (":) later", ":) later"),
            ("bye :) ", "bye :) "),
            ("http://example.com", "http://example.com"),
            ("smile:)", "smile:)"),
            ("", ""),
        ]
        for row in rows {
            XCTAssertEqual(Emoticons.replacingTrailing(in: row.draft), row.sent, row.draft)
        }
    }
}
