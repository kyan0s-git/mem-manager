import Foundation
import XCTest
@testable import MemCore

final class PresentationTests: XCTestCase {
    private let g: UInt64 = 1 << 30

    func testSplitCountsCachedFilesAsReady() {
        let s = MemorySplit(total: 16 * g, used: 9 * g, cached: 5 * g)
        XCTAssertEqual(s.apps, 9 * g)
        XCTAssertEqual(s.cache, 5 * g)
        XCTAssertEqual(s.free, 2 * g)
        XCTAssertEqual(s.ready, 7 * g)
        let clamped = MemorySplit(total: 4 * g, used: 3 * g, cached: 5 * g)
        XCTAssertEqual(clamped.cache, g)
        XCTAssertEqual(clamped.free, 0)
        XCTAssertEqual(MemorySplit(total: 0, used: 0, cached: 0).appsFraction, 0)
    }

    func testNames() {
        XCTAssertEqual(Presentation.stateName(.normal, friendly: true), "Comfortable")
        XCTAssertEqual(Presentation.stateName(.high, friendly: true), "Tight")
        XCTAssertEqual(Presentation.stateName(.elevated, friendly: false), "Elevated")
    }

    func testRoomMade() {
        let r = Presentation.roomMade(first: (0.4, 0.3), last: (0.5, 0.2), total: 10 * g)
        XCTAssertEqual(Double(r), Double(g), accuracy: Double(g) * 0.01)
        XCTAssertEqual(Presentation.roomMade(first: (0.4, 0.3), last: (0.3, 0.2), total: 10 * g), 0)
        XCTAssertNil(Presentation.roomLine(bytes: 64 << 20, windowMs: 600_000))
        XCTAssertEqual(Presentation.roomLine(bytes: g + g / 5, windowMs: 600_000),
                       "Cache made room for 1.2 GB of app memory in the last 10 minutes.")
    }

    func testNudgePreviewOnlyWhenComfortableAndFriendly() {
        let s = MemorySplit(total: 16 * g, used: 9 * g, cached: 5 * g)
        XCTAssertNotNil(Presentation.nudgePreview(.normal, split: s, friendly: true))
        XCTAssertNil(Presentation.nudgePreview(.normal, split: s, friendly: false))
        XCTAssertNil(Presentation.nudgePreview(.high, split: s, friendly: true))
    }

    func testActivityText() {
        XCTAssertEqual(Presentation.appsList(["A", "B"]), "A, B")
        XCTAssertEqual(Presentation.appsList(["A", "B", "C", "D", "E"]), "A, B, C +2")
        XCTAssertEqual(Presentation.resultText(freed: g, refaultRatio: 0, pending: false).0, "1.0 GB back · no slowdown")
        XCTAssertEqual(Presentation.resultText(freed: 0, refaultRatio: 0.5, pending: false).1, .warn)
        XCTAssertEqual(Presentation.summary(actions: 0, freed: 0, slowdowns: 0), "No actions yet.")
        XCTAssertEqual(Presentation.summary(actions: 3, freed: g, slowdowns: 0), "3 actions · 1.0 GB back · no slowdowns")
        XCTAssertEqual(Presentation.nowLine(paused: false, acting: false, state: .normal), "Watching · nothing to do")
        XCTAssertEqual(Presentation.nowLine(paused: true, acting: true, state: .high), "Working on it…")
        XCTAssertEqual(Presentation.markerPos(ageMs: 300_000, windowMs: 600_000), 0.5)
        XCTAssertNil(Presentation.markerPos(ageMs: 700_000, windowMs: 600_000))
    }

    func testVersions() {
        func v(_ s: String) -> AppVersion { AppVersion(s)! }
        XCTAssertTrue(v("v0.2.0") > v("0.1.9"))
        XCTAssertTrue(v("1.0.0") > v("1.0.0-rc1"))
        XCTAssertTrue(v("1.0.0-rc2") > v("1.0.0-rc1"))
        XCTAssertEqual(v("0.1"), v("0.1.0"))
        XCTAssertNil(AppVersion("x.y"))
        XCTAssertNil(AppVersion("1.2.3.4"))
        XCTAssertTrue(v("0.3.0").isPrerelease)
        XCTAssertFalse(v("1.0.0").isPrerelease)
        XCTAssertEqual(v("v1.2.3-beta").description, "1.2.3-beta")
    }

    func testChooseNewestWithChecksummedAsset() throws {
        let json = """
        [
          {"tag_name":"v0.3.0-rc1","draft":false,"prerelease":true,"html_url":"https://x/a",
           "assets":[{"name":"MemManager-0.3.0-rc1-macos-universal.zip","browser_download_url":"https://d/a","size":10},
                     {"name":"MemManager-0.3.0-rc1-macos-universal.zip.sha256","browser_download_url":"https://d/a.sha256","size":1}]},
          {"tag_name":"v0.4.0","draft":true,"prerelease":true,"html_url":"","assets":[]},
          {"tag_name":"v0.2.0","draft":false,"prerelease":true,"html_url":"https://x/b",
           "assets":[{"name":"MemManager-0.2.0-macos-universal.zip","browser_download_url":"https://d/b","size":20}]}
        ]
        """
        let rel = try XCTUnwrap(Updates.parse(Data(json.utf8)))
        XCTAssertEqual(rel.count, 3)
        let name = { (v: AppVersion) in "MemManager-\(v)-macos-universal.zip" }
        let o = try XCTUnwrap(Updates.choose(rel, current: AppVersion("0.1.0")!, assetName: name))
        XCTAssertEqual(o.version.description, "0.3.0-rc1")
        XCTAssertEqual(o.checksum.url, "https://d/a.sha256")
        XCTAssertNil(Updates.choose(rel, current: AppVersion("0.3.0")!, assetName: name))
        XCTAssertNil(Updates.choose(rel, current: AppVersion("1.0.0")!, assetName: name))
        XCTAssertNil(Updates.parse(Data("{}".utf8)))
    }

    func testChecksumFiles() {
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        XCTAssertEqual(Updates.checksum(in: "\(h)  abc.txt\n"), h)
        XCTAssertEqual(Updates.checksum(in: h.uppercased()), h)
        XCTAssertNil(Updates.checksum(in: "nope"))
        XCTAssertNil(Updates.checksum(in: ""))
    }
}
