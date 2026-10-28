import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

/// Layout rules for iPhone Duo poses. Sizes are the inner display in points with a
/// 40-point fold band through the centre, as the Xcode 27.1 simulator reports them.
final class FoldableLayoutTests: XCTestCase {
    private let innerPortrait = CGSize(width: 669, height: 951)
    private let innerLandscape = CGSize(width: 951, height: 669)
    private let bookFold = FoldDivision(frame: CGRect(x: 314.5, y: 0, width: 40, height: 951))
    private let laptopFold = FoldDivision(frame: CGRect(x: 0, y: 314.5, width: 951, height: 40))

    func testFlatDisplaysKeepTheOrdinaryLayout() {
        XCTAssertEqual(FoldAwareChatLayout.arrangement(for: nil, in: innerPortrait), .continuous)
        XCTAssertEqual(FoldAwareChatLayout.mediaArrangement(for: nil, in: innerPortrait), .continuous)
        XCTAssertNil(FoldAwareChatLayout.arrangement(for: nil, in: innerPortrait).bubbleMaxWidth)
        XCTAssertNil(FoldAwareChatLayout.galleryGrid(width: 669, division: nil).gutterAfterColumn)
    }

    func testPanesFollowTheFoldFrame() {
        XCTAssertEqual(bookFold.axis, .vertical)
        XCTAssertEqual(bookFold.leadingPane(in: innerPortrait), CGRect(x: 0, y: 0, width: 314.5, height: 951))
        XCTAssertEqual(bookFold.trailingPane(in: innerPortrait), CGRect(x: 354.5, y: 0, width: 314.5, height: 951))
        XCTAssertEqual(bookFold.bandLength(in: innerPortrait), 40, accuracy: 0.001)

        XCTAssertEqual(laptopFold.axis, .horizontal)
        XCTAssertEqual(laptopFold.leadingPane(in: innerLandscape), CGRect(x: 0, y: 0, width: 951, height: 314.5))
        XCTAssertEqual(laptopFold.trailingPane(in: innerLandscape), CGRect(x: 0, y: 354.5, width: 951, height: 314.5))

        // A region can extend past the view that reads it; panes and the band clamp to the view.
        let overflowing = FoldDivision(frame: CGRect(x: -6, y: 300, width: 963, height: 40))
        XCTAssertEqual(overflowing.axis, .horizontal)
        XCTAssertEqual(overflowing.leadingPane(in: innerLandscape).height, 300, accuracy: 0.001)
        XCTAssertEqual(overflowing.trailingPane(in: innerLandscape).minY, 340, accuracy: 0.001)
        XCTAssertEqual(overflowing.bandLength(in: innerLandscape), 40, accuracy: 0.001)
    }

    func testLaptopPoseKeepsTheTimelineAboveTheFoldAndTheComposerBelow() throws {
        let arrangement = FoldAwareChatLayout.arrangement(for: laptopFold, in: innerLandscape)
        guard case let .stacked(lowerRegionHeight, foldBand) = arrangement else {
            return XCTFail("Expected a stacked arrangement, got \(arrangement)")
        }
        XCTAssertEqual(foldBand, 40, accuracy: 0.001)
        XCTAssertEqual(lowerRegionHeight, 354.5, accuracy: 0.001)
        // The timeline ends exactly where the fold band starts.
        XCTAssertEqual(innerLandscape.height - lowerRegionHeight, laptopFold.frame.minY, accuracy: 0.001)
        XCTAssertNil(arrangement.bubbleMaxWidth)
    }

    func testBookPoseKeepsBubblesAndTheComposerOnOnePane() throws {
        let arrangement = FoldAwareChatLayout.arrangement(for: bookFold, in: innerPortrait)
        guard case let .sideBySide(paneWidth, bubbleMaxWidth) = arrangement else {
            return XCTFail("Expected a side-by-side arrangement, got \(arrangement)")
        }
        XCTAssertEqual(paneWidth, 314.5, accuracy: 0.001)
        XCTAssertEqual(arrangement.bubbleMaxWidth, bubbleMaxWidth)
        // A bubble that starts at the timeline padding never reaches the band.
        let bubbleTrailingEdge = 12 + bubbleMaxWidth
        XCTAssertLessThanOrEqual(bubbleTrailingEdge, bookFold.frame.minX - FoldAwareChatLayout.foldClearance + 0.001)
        XCTAssertGreaterThanOrEqual(bubbleMaxWidth, 120)
    }

    func testFoldsThatLeaveNoUsablePaneFallBackToTheOrdinaryLayout() {
        // With the keyboard up the surface shrinks until the lower pane cannot hold the composer.
        XCTAssertEqual(FoldAwareChatLayout.arrangement(for: laptopFold, in: CGSize(width: 951, height: 400)), .continuous)
        // A fold near the top edge leaves no readable timeline above it.
        let highFold = FoldDivision(frame: CGRect(x: 0, y: 60, width: 951, height: 40))
        XCTAssertEqual(FoldAwareChatLayout.arrangement(for: highFold, in: innerLandscape), .continuous)
        // Narrow book-pose panes, such as a single split-view column, keep bubbles at their natural width.
        let narrowFold = FoldDivision(frame: CGRect(x: 180, y: 0, width: 40, height: 600))
        XCTAssertEqual(FoldAwareChatLayout.arrangement(for: narrowFold, in: CGSize(width: 400, height: 600)), .continuous)
        XCTAssertEqual(FoldAwareChatLayout.mediaArrangement(for: narrowFold, in: CGSize(width: 400, height: 600)), .continuous)
        // Degenerate sizes never divide.
        XCTAssertEqual(FoldAwareChatLayout.arrangement(for: bookFold, in: .zero), .continuous)
    }

    func testMediaViewersSplitAroundTheFold() {
        XCTAssertEqual(
            FoldAwareChatLayout.mediaArrangement(for: laptopFold, in: innerLandscape),
            .stacked(mediaHeight: 314.5, foldBand: 40)
        )
        XCTAssertEqual(
            FoldAwareChatLayout.mediaArrangement(for: bookFold, in: innerPortrait),
            .sideBySide(mediaWidth: 314.5, foldBand: 40)
        )
    }

    func testGalleryGridUsesEvenColumnsWithAGutterOverTheFold() {
        let plan = FoldAwareChatLayout.galleryGrid(width: 669, division: bookFold)
        XCTAssertEqual(plan.columnCount, 4)
        XCTAssertEqual(plan.gutterAfterColumn, 1)
        XCTAssertEqual(plan.gutterWidth, 42, accuracy: 0.001)

        // An odd count becomes even so each pane holds whole tiles.
        let narrowFold = FoldDivision(frame: CGRect(x: 220, y: 0, width: 40, height: 600))
        let narrowPlan = FoldAwareChatLayout.galleryGrid(width: 480, division: narrowFold)
        XCTAssertEqual(narrowPlan.columnCount, 4)
        XCTAssertEqual(narrowPlan.gutterAfterColumn, 1)

        // A laptop-pose fold runs between rows, so columns stay as they are.
        let laptopPlan = FoldAwareChatLayout.galleryGrid(width: 951, division: laptopFold)
        XCTAssertEqual(laptopPlan.columnCount, 6)
        XCTAssertNil(laptopPlan.gutterAfterColumn)

        // Flat displays keep today's density: the outer display, an iPhone, the open inner display.
        XCTAssertEqual(FoldAwareChatLayout.galleryGrid(width: 466, division: nil).columnCount, 3)
        XCTAssertEqual(FoldAwareChatLayout.galleryGrid(width: 402, division: nil).columnCount, 3)
        XCTAssertEqual(FoldAwareChatLayout.galleryGrid(width: 951, division: nil).columnCount, 6)
    }
}
