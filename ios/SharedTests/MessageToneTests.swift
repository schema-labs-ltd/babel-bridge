import AVFoundation
import XCTest
import UserNotifications
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

@MainActor
final class MessageToneTests: XCTestCase {
    #if os(iOS)
    func testCommunicationNotificationPreservesCustomAndSilentSounds() throws {
        let content = UNMutableNotificationContent()
        content.title = "Alex"
        content.body = "Hello"
        content.userInfo = ["contactId": "alex@s.whatsapp.net", "senderName": "Alex", "messageBody": "Hello"]
        content.sound = UNNotificationSound(named: UNNotificationSoundName("bb-aurora.wav"))
        let audible = NotificationMessagePresentation.messagingContent(content, avatarData: nil, donate: false)
        XCTAssertEqual(audible.sound, content.sound)
        content.sound = nil
        let silent = NotificationMessagePresentation.messagingContent(content, avatarData: nil, donate: false)
        XCTAssertNil(silent.sound)
        XCTAssertEqual(silent.body, "Hello")
    }
    #endif

    func testEveryBundledToneCanBeDecoded() throws {
        XCTAssertEqual(Set(MessageTone.catalog.map(\.id)), Set(MessageTone.customTones))
        for tone in MessageTone.customTones {
            let url = try XCTUnwrap(tone.resourceURL(), tone.rawValue)
            let descriptor = try XCTUnwrap(MessageTone.catalog.first { $0.id == tone })
            XCTAssertEqual(descriptor.filename, tone.filename)
            // Decodes to PCM without touching audio hardware, so it runs in CI.
            let file = try AVAudioFile(forReading: url)
            let duration = Double(file.length) / file.processingFormat.sampleRate
            XCTAssertTrue((0.2..<3).contains(duration), tone.title)
            XCTAssertEqual(file.processingFormat.channelCount, 1)
            let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
            try file.read(into: buffer)
            XCTAssertGreaterThan(buffer.frameLength, 0, tone.title)
        }
    }

    private func skipWithoutAudioPlayback() throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["SKIP_AUDIO_PLAYBACK"] == "1",
            "Audio playback is checked locally and on devices, not in CI."
        )
    }

    /// Needs a working audio output, which a cold CI simulator does not reliably have.
    /// CI sets `TEST_RUNNER_SKIP_AUDIO_PLAYBACK=1`; run it locally or on a device.
    func testEveryBundledToneActuallyPlays() async throws {
        try skipWithoutAudioPlayback()
        for tone in MessageTone.customTones {
            let url = try XCTUnwrap(tone.resourceURL(), tone.rawValue)
            let audio = try AVAudioPlayer(contentsOf: url)
            XCTAssertTrue(audio.prepareToPlay())
            XCTAssertTrue(audio.play(), tone.title)
            // A cold simulator audio session can take a moment to start the first tone.
            for _ in 0..<40 where !(audio.isPlaying && audio.currentTime > 0) {
                try await Task.sleep(for: .milliseconds(25))
            }
            XCTAssertTrue(audio.isPlaying, tone.title)
            XCTAssertGreaterThan(audio.currentTime, 0)
            audio.stop()
        }
    }

    /// Plays real audio, so CI skips it like the test above.
    func testPreviewCanSwitchTonesAndStopsForSilentOrDismissal() throws {
        try skipWithoutAudioPlayback()
        let player = MessageTonePreviewPlayer()
        try player.play(.aurora)
        XCTAssertTrue(player.isPlaying)
        XCTAssertEqual(player.playingTone, .aurora)
        try player.play(.bamboo)
        XCTAssertEqual(player.playingTone, .bamboo)
        try player.play(.silent)
        XCTAssertFalse(player.isPlaying)
        XCTAssertNil(player.playingTone)
        try player.play(.orbit)
        player.stop()
        XCTAssertFalse(player.isPlaying)
        XCTAssertNil(player.playingTone)
    }

    func testInheritIsEncodedAsExplicitNullAndSilentAsItsOwnChoice() throws {
        let inherit = try JSONSerialization.jsonObject(with: JSONEncoder().encode(MessageToneUpdate(tone: nil))) as? [String: Any]
        XCTAssertTrue(inherit?["tone"] is NSNull)
        let silent = try JSONSerialization.jsonObject(with: JSONEncoder().encode(MessageToneUpdate(tone: .silent))) as? [String: Any]
        XCTAssertEqual(silent?["tone"] as? String, "silent")
        let response = try JSONDecoder().decode(MessageToneSettings.self, from: Data(#"{"tone":null,"globalTone":"glass","effectiveTone":"glass"}"#.utf8))
        XCTAssertNil(response.tone)
        XCTAssertEqual(response.effectiveTone, .glass)
    }

    func testRingtoneEndpointPreservesExactConversationIdentifier() throws {
        let id = "family+one&two%three@g.us"
        let path = APIClient.messageTonePath(contactID: id)
        XCTAssertTrue(path.contains("%2B"))
        let components = try XCTUnwrap(URLComponents(string: path))
        XCTAssertEqual(components.queryItems, [URLQueryItem(name: "contactId", value: id)])
        XCTAssertEqual(APIClient.messageTonePath(contactID: nil), "/api/settings/message-tone")
    }

    func testConversationOverridesStayIndependentAndCanReturnToGlobal() async throws {
        let session = AppSession(demoMode: true)
        _ = try await session.saveMessageTone(.aurora, contactID: nil)
        _ = try await session.saveMessageTone(.bamboo, contactID: "family@g.us")
        _ = try await session.saveMessageTone(.silent, contactID: "quiet@g.us")
        _ = try await session.saveMessageTone(.glass, contactID: nil)
        var family = try await session.messageToneSettings(contactID: "family@g.us")
        XCTAssertEqual(family.effectiveTone, .bamboo)
        let quiet = try await session.messageToneSettings(contactID: "quiet@g.us")
        XCTAssertEqual(quiet.effectiveTone, .silent)
        family = try await session.saveMessageTone(nil, contactID: "family@g.us")
        XCTAssertNil(family.tone)
        XCTAssertEqual(family.effectiveTone, .glass)
        let other = try await session.messageToneSettings(contactID: "other@g.us")
        XCTAssertNil(other.tone)
        XCTAssertEqual(other.effectiveTone, .glass)
    }
}
