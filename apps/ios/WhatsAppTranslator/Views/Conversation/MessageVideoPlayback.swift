import AVKit
import Foundation
import SwiftUI

#if os(macOS)
struct InlineVideoPlayer: View {
    let url: URL
    @State private var player: AVPlayer

    init(url: URL) {
        self.url = url
        _player = State(initialValue: AVPlayer(url: url))
    }

    var body: some View {
        MacInlineVideoPlayer(player: player)
            .frame(width: 280, height: 190)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .onDisappear { player.pause() }
    }
}

private struct MacInlineVideoPlayer: NSViewRepresentable {
    let player: AVPlayer

    func makeNSView(context: Context) -> AVPlayerView {
        let playerView = AVPlayerView()
        playerView.player = player
        playerView.controlsStyle = .inline
        playerView.videoGravity = .resizeAspect
        return playerView
    }

    func updateNSView(_ playerView: AVPlayerView, context: Context) {
        if playerView.player !== player {
            playerView.player = player
        }
    }

    static func dismantleNSView(_ playerView: AVPlayerView, coordinator: Void) {
        playerView.player?.pause()
        playerView.player = nil
    }
}

#else
struct InlineVideoPlayer: View {
    @State private var playback: MessageVideoPlayback

    init(url: URL) {
        _playback = State(initialValue: MessageVideoPlayback(url: url))
    }

    var body: some View {
        InlineVideoPreview(playback: playback)
            .frame(width: 280, height: 190)
            .overlay {
                Image(systemName: "play.circle.fill")
                    .font(.system(size: 48))
                    .foregroundStyle(.white.opacity(0.9))
                    .shadow(radius: 4)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .contentShape(Rectangle())
            .highPriorityGesture(
                TapGesture().onEnded { playback.presentFullScreen() }
            )
            .accessibilityLabel("Play video full screen")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { playback.presentFullScreen() }
    }
}

/// The message, full-screen player and PiP window share one player and playhead.
/// AVKit owns background transitions; removing a bubble must not stop its PiP window.
@MainActor
final class MessageVideoPlayback: NSObject {
    let player: AVPlayer
    private var rateObservation: NSKeyValueObservation?
    private(set) var isAttached = false
    private(set) var isFullScreen = false
    private(set) var isPictureInPicture = false
    private static weak var playing: MessageVideoPlayback?
    private static var presentations: [ObjectIdentifier: MessageVideoPlayback] = [:]

    let controller = AVPlayerViewController()

    init(url: URL) {
        player = AVPlayer(url: url)
        super.init()
        controller.player = player
        controller.delegate = self
        controller.showsPlaybackControls = true
        controller.allowsPictureInPicturePlayback = true
        controller.canStartPictureInPictureAutomaticallyFromInline = true
        rateObservation = player.observe(\.rate, options: [.new]) { [weak self] player, _ in
            guard player.rate > 0 else { return }
            Task { @MainActor [weak self] in self?.didStartPlayback() }
        }
    }

    private func didStartPlayback() {
        guard player.rate > 0 else { return }
        if Self.playing !== self { Self.playing?.player.pause() }
        Self.playing = self
        do {
            // Activate only when the user plays a video, never when a bubble loads.
            let audio = AVAudioSession.sharedInstance()
            try audio.setCategory(.playback, mode: .moviePlayback)
            try audio.setActive(true)
        } catch {
            player.pause()
        }
    }

    func attach() { isAttached = true }

    /// Tapping a video bubble opens the shared controller full screen and starts playback.
    func presentFullScreen(animated: Bool = true, completion: ((Bool) -> Void)? = nil) {
        guard controller.presentingViewController == nil,
              let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first(where: { $0.activationState == .foregroundActive }),
              var presenter = scene.windows.first(where: \.isKeyWindow)?.rootViewController else {
            completion?(controller.presentingViewController != nil)
            return
        }
        while let presented = presenter.presentedViewController { presenter = presented }
        setFullScreen(true)
        controller.modalPresentationStyle = .fullScreen
        presenter.present(controller, animated: animated) { [weak self] in
            if animated { self?.player.play() }
            completion?(true)
        }
    }

    func detach() {
        isAttached = false
        finishPresentationIfNeeded()
    }

    func setFullScreen(_ active: Bool) {
        isFullScreen = active
        finishPresentationIfNeeded()
    }

    func setPictureInPicture(_ active: Bool) {
        isPictureInPicture = active
        finishPresentationIfNeeded()
    }

    private func finishPresentationIfNeeded() {
        if isFullScreen || isPictureInPicture {
            Self.presentations[ObjectIdentifier(self)] = self
        } else {
            if !isAttached { player.pause() }
            Self.presentations[ObjectIdentifier(self)] = nil
        }
    }

    static func stopAll() {
        playing?.player.pause()
        for playback in presentations.values { playback.player.replaceCurrentItem(with: nil) }
        presentations.removeAll()
        playing = nil
    }
}

private final class PlayerLayerView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
}

/// The bubble shows the shared player's frames; the controls live in the full-screen controller.
private struct InlineVideoPreview: UIViewRepresentable {
    let playback: MessageVideoPlayback
    func makeCoordinator() -> MessageVideoPlayback { playback }
    func makeUIView(context: Context) -> PlayerLayerView {
        playback.attach()
        let view = PlayerLayerView()
        view.backgroundColor = .black
        view.playerLayer.videoGravity = .resizeAspect
        view.playerLayer.player = playback.player
        return view
    }
    func updateUIView(_ view: PlayerLayerView, context: Context) {}
    static func dismantleUIView(_ view: PlayerLayerView, coordinator: MessageVideoPlayback) {
        view.playerLayer.player = nil
        coordinator.detach()
    }
}

extension MessageVideoPlayback: @preconcurrency AVPlayerViewControllerDelegate {
    func playerViewController(_ playerViewController: AVPlayerViewController, willBeginFullScreenPresentationWithAnimationCoordinator coordinator: any UIViewControllerTransitionCoordinator) {
        setFullScreen(true)
        coordinator.animate(alongsideTransition: nil) { [weak self] context in
            if context.isCancelled { self?.setFullScreen(false) }
        }
    }
    func playerViewController(_ playerViewController: AVPlayerViewController, willEndFullScreenPresentationWithAnimationCoordinator coordinator: any UIViewControllerTransitionCoordinator) {
        coordinator.animate(alongsideTransition: nil) { [weak self] context in
            if !context.isCancelled { self?.setFullScreen(false) }
        }
    }
    func playerViewControllerWillStartPictureInPicture(_ playerViewController: AVPlayerViewController) {
        setPictureInPicture(true)
    }
    func playerViewControllerDidStopPictureInPicture(_ playerViewController: AVPlayerViewController) {
        setPictureInPicture(false)
    }
    func playerViewController(_ playerViewController: AVPlayerViewController, failedToStartPictureInPictureWithError error: Error) {
        setPictureInPicture(false)
    }
    func playerViewControllerShouldAutomaticallyDismissAtPictureInPictureStart(_ playerViewController: AVPlayerViewController) -> Bool { false }
    func playerViewController(_ playerViewController: AVPlayerViewController, restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void) {
        if playerViewController.presentingViewController != nil {
            completionHandler(true)
            return
        }
        // The user may have left the conversation while the PiP window was playing.
        presentFullScreen(animated: false, completion: completionHandler)
    }
}
#endif
