import Foundation
import Testing
@testable import GroveApp

struct NotionPageLinkTests {
    private let id = "01234567-89ab-cdef-0123-456789abcdef"

    @Test(arguments: [
        "https://app.notion.com/p/team/0123456789abcdef0123456789abcdef?source=copy_link",
        "https://app.notion.com/p/other-team/0123456789abcdef0123456789abcdef",
        "https://app.notion.com/p/Page-0123456789abcdef0123456789abcdef",
        "https://app.notion.com/p/0123456789abcdef0123456789abcdef",
        "https://www.notion.so/Page-0123456789abcdef0123456789abcdef?v=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "https://workspace.notion.so/회의-0123456789abcdef0123456789abcdef#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "https://notion.com/workspace/0123456789abcdef0123456789abcdef?v=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "https://www.notion.com/workspace/0123456789abcdef0123456789abcdef",
        "https://workspace.notion.site/Page-0123456789abcdef0123456789abcdef?source=copy_link",
        "notion://app.notion.com/p/team/0123456789abcdef0123456789abcdef",
        "notion://www.notion.so/0123456789abcdef0123456789abcdef",
        "https://APP.NOTION.COM:443/p/0123456789ABCDEF0123456789ABCDEF/",
        "https://app.notion.com/p/%ED%9A%8C%EC%9D%98-01234567-89ab-cdef-0123-456789abcdef",
        "  0123456789ABCDEF0123456789ABCDEF\n",
        "01234567-89AB-CDEF-0123-456789ABCDEF"
    ])
    func supportedReferencesKeepThePageIdentity(_ input: String) throws {
        #expect(try NotionPageLink.id(from: input) == id)
        #expect(NotionPageLink.validationMessage(for: input) == nil)
        #expect(NotionPageLink.url(for: id).absoluteString == "https://app.notion.com/p/0123456789abcdef0123456789abcdef")
    }

    @Test(arguments: [
        "https://app.notion.com/p/team",
        "https://workspace.notion.site/meeting-notes",
        "https://workspace.notion.site/",
        "https://app.notion.com/p/Page-00123456789abcdef0123456789abcdef",
        "https://app.notion.com/p/01234567-89ab-cdef-0123-456789abcdeg",
        "https://app.notion.com/p/not-a-page-id"
    ])
    func missingOrDamagedIDsNeverBecomeAnotherPage(_ input: String) {
        #expect(throws: NotionExportError.self) { try NotionPageLink.id(from: input) }
        #expect(NotionPageLink.validationMessage(for: input) != nil)
    }

    @Test(arguments: [
        "https://app.notion.com/p/0123456789abcdef0123456789abcdef?p=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "https://app.notion.com/p/0123456789abcdef0123456789abcdef?peek=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "https://app.notion.com/p/0123456789abcdef0123456789abcdef?p=0123456789abcdef0123456789abcdef&p=0123456789abcdef0123456789abcdef"
    ])
    func conflictingPeekTargetsAreNotSilentlyIgnored(_ input: String) {
        #expect(throws: NotionExportError.self) { try NotionPageLink.id(from: input) }
    }

    @Test(arguments: [
        "https://app.notion.com.evil.test/0123456789abcdef0123456789abcdef",
        "https://fakeapp.notion.com/0123456789abcdef0123456789abcdef",
        "https://www.example.com/0123456789abcdef0123456789abcdef",
        "https://user:password@app.notion.com/p/0123456789abcdef0123456789abcdef",
        "https://app.notion.com:8080/p/0123456789abcdef0123456789abcdef",
        "http://app.notion.com/p/0123456789abcdef0123456789abcdef",
        "javascript:alert(1)", "file:///0123456789abcdef0123456789abcdef",
        "notion://docs/enhanced-markdown-spec",
        "collection://01234567-89ab-cdef-0123-456789abcdef",
        "view://01234567-89ab-cdef-0123-456789abcdef"
    ])
    func nonPageResourcesAndOtherSitesDoNotBecomeDestinations(_ input: String) {
        #expect(throws: NotionExportError.self) { try NotionPageLink.id(from: input) }
    }

    @Test func emptyFieldsHaveNoValidationNotice() {
        #expect(NotionPageLink.validationMessage(for: " \n") == nil)
    }

    @Test func canonicalServerURLsAndDatabaseItemTitlesAreAccepted() throws {
        let response: [String: Any] = ["metadata": ["type": "page"], "title": "항목 제목",
            "text": "<page url=\"https://app.notion.com/p/0123456789abcdef0123456789abcdef\"><properties>{\"Name\":\"항목 제목\"}</properties><content>본문</content></page>"]
        let page = try NotionFetchedPage(response: response, expectedID: id)
        #expect(page.title == "항목 제목" && page.id == id)
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: response, expectedID: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee") }
    }

    @Test func explicitBlankPagesHaveAnEmptyBody() throws {
        let text = "Here is the result of fetch:\n<page url=\"https://app.notion.com/p/0123456789abcdef0123456789abcdef\"><properties>{\"title\":\"빈 페이지\"}</properties><blank-page>This page is blank and has no content.</blank-page></page>"
        let page = try NotionFetchedPage(response: ["text": text], expectedID: id)
        #expect(page.id == id && page.title == "빈 페이지" && page.content.isEmpty)
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["text": text, "truncated": true], expectedID: id) }
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["text": text, "unknown_block_count": 1], expectedID: id) }
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["text": text], expectedID: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee") }
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["text": String(text.dropLast(7))], expectedID: id) }
    }

    @Test(arguments: [
        "", "<blank-page>Unclosed", "<content>Unclosed",
        "<content>Unclosed<blank-page>Empty</blank-page>",
        "<content>Body</content><blank-page>Empty</blank-page>"
    ])
    func missingOrConflictingBodiesAreNotTreatedAsBlank(_ body: String) {
        let text = "<page url=\"https://app.notion.com/p/0123456789abcdef0123456789abcdef\"><properties>{\"title\":\"제목\"}</properties>\(body)</page>"
        #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["text": text], expectedID: id) }
    }

    @Test func serverEntityKindsAreValidatedSeparatelyFromTheURL() {
        for kind in ["database", "data_source", "view", "folder"] {
            #expect(throws: NotionExportError.self) { try NotionFetchedPage(response: ["metadata": ["type": kind], "text": ""], expectedID: id) }
        }
    }
}
