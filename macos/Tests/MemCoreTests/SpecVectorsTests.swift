import Foundation
import XCTest
@testable import MemCore

/// Ensures the shared spec vectors are reachable from the Swift test target.
/// Algorithm assertions are added together with the MemCore implementation.
final class SpecVectorsTests: XCTestCase {
    func testVectorsArePresentAndParse() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // MemCoreTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // macos
            .deletingLastPathComponent()   // repo root
            .appendingPathComponent("spec/test-vectors")
        let files = try FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
        XCTAssertFalse(files.isEmpty)
        for file in files {
            XCTAssertNoThrow(try JSONSerialization.jsonObject(with: Data(contentsOf: file)), file.lastPathComponent)
        }
        XCTAssertEqual(MemCore.specVersion, 1)
    }
}
