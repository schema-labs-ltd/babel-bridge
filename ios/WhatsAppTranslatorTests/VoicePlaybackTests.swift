import AVFoundation
import XCTest
@testable import WhatsAppTranslator

final class VoicePlaybackTests: XCTestCase {
    func testConvertedWhatsAppAudioDecodes() throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "playback-fixture", withExtension: "mp3"))
        // Decodes to PCM without touching audio hardware, so it runs in CI.
        let file = try AVAudioFile(forReading: url)
        XCTAssertGreaterThan(Double(file.length) / file.processingFormat.sampleRate, 0.9)
        XCTAssertGreaterThan(file.processingFormat.channelCount, 0)
        let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
        try file.read(into: buffer)
        XCTAssertGreaterThan(buffer.frameLength, 0)
    }

    /// Needs a working audio output, which a cold CI simulator does not reliably have.
    /// CI sets `TEST_RUNNER_SKIP_AUDIO_PLAYBACK=1`; run it locally or on a device.
    @MainActor
    func testConvertedWhatsAppAudioActuallyStartsAndAdvances() async throws {
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["SKIP_AUDIO_PLAYBACK"] == "1",
            "Audio playback is checked locally and on devices, not in CI."
        )
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "playback-fixture", withExtension: "mp3"))
        let player = try AVAudioPlayer(contentsOf: url)
        #if os(iOS)
        try AVAudioSession.sharedInstance().setCategory(.playback, mode: .default)
        try AVAudioSession.sharedInstance().setActive(true)
        #endif
        defer { player.stop() }
        XCTAssertTrue(player.play())
        try await Task.sleep(for: .milliseconds(250))
        XCTAssertTrue(player.isPlaying)
        XCTAssertGreaterThan(player.currentTime, 0.05)
    }
}
