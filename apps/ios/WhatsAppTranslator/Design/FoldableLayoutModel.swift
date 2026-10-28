import Foundation

// Layout rules for folding displays, kept free of SwiftUI so they run in unit tests on every platform.
//
// iPhone Duo reports its hinge as a reserved "division" region while the device is partially open.
// The rules below follow Apple's guidance to move only what is necessary: on a flat display, a
// closed device or an ordinary iPhone every surface keeps its usual layout.

// MARK: - Fold model

/// The hinge of a folding display, in the coordinate space of the view that read it.
///
/// The frame already includes the system's margins around the physical fold, so content that
/// stays outside it remains readable and tappable.
struct FoldDivision: Equatable {
    enum Axis: Equatable {
        /// The fold runs top to bottom (book pose): leading and trailing panes.
        case vertical
        /// The fold runs side to side (laptop pose): upper and lower panes.
        case horizontal
    }

    let frame: CGRect

    var axis: Axis { frame.height >= frame.width ? .vertical : .horizontal }

    /// The pane before the fold in reading order: the left pane in book pose or the upper pane in laptop pose.
    func leadingPane(in size: CGSize) -> CGRect {
        switch axis {
        case .vertical:
            return CGRect(x: 0, y: 0, width: max(0, min(frame.minX, size.width)), height: size.height)
        case .horizontal:
            return CGRect(x: 0, y: 0, width: size.width, height: max(0, min(frame.minY, size.height)))
        }
    }

    /// The pane after the fold: the right pane in book pose or the lower pane in laptop pose.
    func trailingPane(in size: CGSize) -> CGRect {
        switch axis {
        case .vertical:
            let start = min(max(0, frame.maxX), size.width)
            return CGRect(x: start, y: 0, width: size.width - start, height: size.height)
        case .horizontal:
            let start = min(max(0, frame.maxY), size.height)
            return CGRect(x: 0, y: start, width: size.width, height: size.height - start)
        }
    }

    /// The thickness of the fold band, clamped to the part that overlaps the view.
    func bandLength(in size: CGSize) -> CGFloat {
        switch axis {
        case .vertical: return max(0, min(frame.maxX, size.width) - max(frame.minX, 0))
        case .horizontal: return max(0, min(frame.maxY, size.height) - max(frame.minY, 0))
        }
    }
}

// MARK: - Layout policy

/// Pure layout decisions for chat surfaces on a partially folded display.
enum FoldAwareChatLayout {
    /// The smallest pane that still shows a useful run of messages.
    static let minimumTimelineLength: CGFloat = 160
    /// The smallest lower pane that fits the resting composer below the fold.
    static let minimumComposerPane: CGFloat = 132
    /// The narrowest book-pose pane that can hold bubbles without wrapping every word.
    static let minimumBubblePane: CGFloat = 200
    /// Air kept between a bubble and the fold band.
    static let foldClearance: CGFloat = 12

    enum Arrangement: Equatable {
        /// Nothing divides the surface; use the ordinary layout.
        case continuous
        /// Laptop pose: the timeline reads on the upper pane and the composer rests on the lower pane.
        /// `lowerRegionHeight` spans the fold band plus the lower pane; `foldBand` is the band itself.
        case stacked(lowerRegionHeight: CGFloat, foldBand: CGFloat)
        /// Book pose: bubbles stay within one pane and the composer sits on the trailing pane.
        case sideBySide(paneWidth: CGFloat, bubbleMaxWidth: CGFloat)

        var bubbleMaxWidth: CGFloat? {
            if case let .sideBySide(_, width) = self { return width }
            return nil
        }
    }

    static func arrangement(
        for division: FoldDivision?,
        in size: CGSize,
        horizontalPadding: CGFloat = 12
    ) -> Arrangement {
        guard let division, size.width > 0, size.height > 0 else { return .continuous }
        let leading = division.leadingPane(in: size)
        let trailing = division.trailingPane(in: size)
        let band = division.bandLength(in: size)
        switch division.axis {
        case .horizontal:
            guard leading.height >= minimumTimelineLength, trailing.height >= minimumComposerPane else {
                return .continuous
            }
            return .stacked(lowerRegionHeight: trailing.height + band, foldBand: band)
        case .vertical:
            let paneWidth = min(leading.width, trailing.width)
            guard paneWidth >= minimumBubblePane else { return .continuous }
            return .sideBySide(
                paneWidth: paneWidth,
                bubbleMaxWidth: max(120, paneWidth - horizontalPadding - foldClearance)
            )
        }
    }

    enum MediaArrangement: Equatable {
        /// Media above its controls, as on every flat display.
        case continuous
        /// Laptop pose: media fills the upper pane and the controls fill the lower pane.
        case stacked(mediaHeight: CGFloat, foldBand: CGFloat)
        /// Book pose: media fills the leading pane and the controls fill the trailing pane.
        case sideBySide(mediaWidth: CGFloat, foldBand: CGFloat)
    }

    /// Where a full-screen media surface and its controls go so neither crosses the fold.
    static func mediaArrangement(for division: FoldDivision?, in size: CGSize) -> MediaArrangement {
        guard let division, size.width > 0, size.height > 0 else { return .continuous }
        let leading = division.leadingPane(in: size)
        let trailing = division.trailingPane(in: size)
        let band = division.bandLength(in: size)
        switch division.axis {
        case .horizontal:
            guard leading.height >= minimumTimelineLength, trailing.height >= minimumComposerPane else {
                return .continuous
            }
            return .stacked(mediaHeight: leading.height, foldBand: band)
        case .vertical:
            guard leading.width >= minimumBubblePane, trailing.width >= minimumBubblePane else {
                return .continuous
            }
            return .sideBySide(mediaWidth: leading.width, foldBand: band)
        }
    }

    /// Columns for a square photo grid, with a gutter over the fold so no tile is split by the hinge.
    struct GridPlan: Equatable {
        static let tileSpacing: CGFloat = 2

        let columnCount: Int
        /// The column whose trailing spacing widens into the fold gutter, if the fold divides the grid.
        let gutterAfterColumn: Int?
        let gutterWidth: CGFloat
    }

    static func galleryGrid(width: CGFloat, division: FoldDivision?) -> GridPlan {
        var count = width < 500 ? 3 : max(4, Int(width / 145))
        guard let division, division.axis == .vertical, width > 0 else {
            return GridPlan(columnCount: count, gutterAfterColumn: nil, gutterWidth: GridPlan.tileSpacing)
        }
        // Apple recommends an even number of columns on a folding display so content divides cleanly.
        if !count.isMultiple(of: 2) { count += 1 }
        let band = division.bandLength(in: CGSize(width: width, height: 1))
        return GridPlan(
            columnCount: count,
            gutterAfterColumn: count / 2 - 1,
            gutterWidth: max(GridPlan.tileSpacing, band + GridPlan.tileSpacing)
        )
    }
}
