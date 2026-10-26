import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

enum MessageSwipeReply {
    static let triggerDistance: CGFloat = 56
    static let maximumReveal: CGFloat = 72

    static func offset(translation: CGSize) -> CGFloat {
        guard translation.width > 0,
              abs(translation.width) > abs(translation.height) else { return 0 }
        return min(translation.width, maximumReveal)
    }

    static func shouldReply(translation: CGSize) -> Bool {
        offset(translation: translation) >= triggerDistance
    }

    static func shouldBegin(velocity: CGPoint) -> Bool {
        velocity.x > 0 && abs(velocity.x) > abs(velocity.y)
    }
}

#if os(iOS)
private struct MessageSwipeReplyPanGesture: UIGestureRecognizerRepresentable {
    let onChanged: (CGSize) -> Void
    let onEnded: (CGSize) -> Void

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard let pan = gestureRecognizer as? UIPanGestureRecognizer,
                  let view = pan.view else { return false }
            return MessageSwipeReply.shouldBegin(velocity: pan.velocity(in: view))
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let recognizer = UIPanGestureRecognizer()
        recognizer.delegate = context.coordinator
        recognizer.cancelsTouchesInView = false
        return recognizer
    }

    func updateUIGestureRecognizer(_ recognizer: UIPanGestureRecognizer, context: Context) {}

    func handleUIGestureRecognizerAction(_ recognizer: UIPanGestureRecognizer, context: Context) {
        guard let view = recognizer.view else { return }
        let translation = recognizer.translation(in: view)
        let size = CGSize(width: translation.x, height: translation.y)
        switch recognizer.state {
        case .began, .changed:
            onChanged(size)
        case .ended:
            onEnded(size)
        case .cancelled, .failed:
            onEnded(.zero)
        default:
            break
        }
    }
}
#endif

struct MessageSwipeReplyModifier: ViewModifier {
    @Binding var translation: CGSize
    let onReply: () -> Void

    func body(content: Content) -> some View {
        #if os(iOS)
        content.gesture(
            MessageSwipeReplyPanGesture(
                onChanged: { translation = $0 },
                onEnded: finish
            )
        )
        #else
        content.simultaneousGesture(
            DragGesture(minimumDistance: 12)
                .onChanged { translation = $0.translation }
                .onEnded { finish($0.translation) }
        )
        #endif
    }

    private func finish(_ finalTranslation: CGSize) {
        let shouldReply = MessageSwipeReply.shouldReply(translation: finalTranslation)
        withAnimation(.snappy) { translation = .zero }
        if shouldReply { onReply() }
    }
}
