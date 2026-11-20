// Help is read by agents, so the general page only names commands and each
// command's page carries its own usage and rules.

struct Topic {
    words: &'static [&'static str],
    usage: &'static str,
    notes: &'static [&'static [&'static str]],
}

const READ: &[&str] = &[
    "조회는 읽기 전용이며 폴더나 파일을 만들지 않습니다.",
    "기본 50개, 최대 200개씩 조회합니다. 다음 페이지는 pagination.nextOffset과 첫 응답의 sourceVersion(--expected-source-version)을 씁니다.",
    "--max-bytes N은 JSON 예산(1 이상 1048576 이하)이며 예산에 따라 limit보다 적게 반환할 수 있습니다.",
];
const DATES: &[&str] = &[
    "날짜 필터는 Unix epoch 밀리초 또는 YYYY-MM-DD이며 from 포함, to 미포함입니다.",
    "문자열 날짜와 --on은 --timezone IANA가 필요합니다. --on과 --from/--to는 함께 쓸 수 없습니다. 생략한 목록 표시는 UTC입니다.",
    "날짜 경계가 없는 날이나 모호한 자정은 거부하며 epoch 경계로 명시할 수 있습니다.",
];
const CHUNKS: &[&str] = &[
    "본문은 개행을 보존한 최대 4096바이트 조각으로 나눕니다. offset/limit은 조각 수이며 페이지를 순서대로 이으면 전체 본문입니다.",
];
const SUMMARY_GET: &[&str] = &[
    "요약은 기본 Markdown이며 내부 인용 번호를 뺍니다. --format txt는 일반 텍스트입니다.",
    "--include-citations는 현재 사용자가 내부 인용 번호를 명시적으로 요청했을 때만 씁니다.",
];
const TRANSCRIPT: &[&str] = &["--start/--end는 초 단위이며 그 구간과 겹치는 발언을 조회합니다."];
const SUMMARY_SAVE: &[&str] = &[
    "summary context로 문맥을 받고, import 파일에는 그 context, 생성한 text, producer를 넣습니다.",
    "원문, 메모, 참석자, 기존 요약이 바뀌었으면 저장하지 않습니다. 저장은 실행 중인 Nero에 요청합니다.",
];
const EDIT: &[&str] = &[
    "변경은 현재 사용자가 요청했을 때만 하며 실행 중인 Nero에 요청합니다.",
    "--expected-version은 조회 응답의 editVersion이며 sourceVersion과 다릅니다.",
];
const MEETING_FILE: &[&str] = &[
    "update 파일은 title/notes, 일괄 작업 파일은 expectedVersions를 담고 move는 folderId(null은 미분류)를 더합니다. 휴지통은 복구할 수 있고 영구 삭제는 없습니다.",
];
const PEOPLE_FILE: &[&str] = &[
    "people update 파일은 people/groups 전체 배열, attendees 파일은 attendees 배열입니다.",
];
const PROMOTE_FILE: &[&str] =
    &["attendee-promote 파일은 attendeeId/name/expectedCatalogVersion 세 문자열입니다."];
const TEMPLATES: &[&str] = &[
    "템플릿 JSON은 name/description/prompt 세 문자열이며 --file -는 stdin을 읽습니다.",
    "기본/사용자 템플릿을 지워도 기존 회의 요약은 남습니다.",
];
const JOBS: &[&str] = &[
    "작업 조회/취소는 실행 중인 Nero에 요청하며 모델을 실행하지 않습니다.",
    "취소는 jobs list의 editVersion을 쓰고, 수락된 뒤 상태를 다시 조회합니다.",
];
const IMPORT: &[&str] = &[
    "실행 중인 Nero가 원본을 복사해 가져오며 원본 파일은 옮기거나 바꾸지 않습니다.",
    "--meeting-id는 음성이 없는 빈 회의에만 붙이며 meetings get의 editVersion이 필요합니다.",
    "가져오기 작업 ID를 반환하므로 jobs list로 진행을 봅니다. 녹음 중이거나 음성 인식 준비 전에는 거부합니다.",
];
const SPEECH: &[&str] = &[
    "준비된 엔진이면 바로 전환하고, 아니면 현재 엔진을 유지한 채 수 GB의 모델을 내려받은 뒤 전환합니다. 현재 사용자가 요청했을 때만 씁니다.",
    "메모리 경고가 있으면 사용자 확인 후에만 --acknowledge-memory-warning을 씁니다. cancel-switch는 대기 중인 전환과 다운로드를 취소합니다.",
    "현재 엔진과 대기 중인 전환은 doctor의 speech에서 봅니다.",
];
const UPDATE: &[&str] = &[
    "실행 중인 Nero의 공식 업데이트 흐름을 씁니다. check는 공개 채널을 조회하고 download는 내려받기를 시작하며 status로 진행을 봅니다.",
    "install은 모든 편집 저장과 녹음/처리 유휴를 확인한 뒤 설치/재시작 요청 수락을 반환합니다. accepted:true는 설치 완료가 아니므로 다시 실행된 Nero의 버전을 확인합니다.",
    "구버전 앱이 이 명령을 모르면 앱의 업데이트 화면을 씁니다.",
];
const DOCTOR: &[&str] = &[
    "저장소가 없어도 실행 중인 연동 버전과 앱 동봉 버전을 비교합니다. 공개 최신 연동은 확인하지 않으며 네트워크 조회나 자동 설치를 하지 않습니다.",
    "speech에는 현재 음성 인식 엔진, 화자 구분 설정, 선택지와 대기 중인 전환이 있습니다.",
];
const MCP: &[&str] = &["stdin/stdout으로 MCP를 제공하며 stdout에 진단 문구를 쓰지 않습니다."];

const TOPICS: &[Topic] = &[
    Topic { words: &["meetings", "list"], usage: "nero meetings list [--trashed] [--folder-id ID | --unfiled] [--from MS|DATE] [--to MS|DATE] [--on DATE] [--timezone IANA] [--offset N] [--limit N]", notes: &[READ, DATES] },
    Topic { words: &["meetings", "search"], usage: "nero meetings search QUERY [--folder-id ID | --unfiled] [--from MS|DATE] [--to MS|DATE] [--on DATE] [--timezone IANA] [--offset N] [--limit N]", notes: &[READ, DATES] },
    Topic { words: &["meetings", "get"], usage: "nero meetings get ID [--offset N] [--limit N]", notes: &[READ, CHUNKS] },
    Topic { words: &["transcript", "get"], usage: "nero transcript get ID [--start SECONDS] [--end SECONDS] [--offset N] [--limit N]", notes: &[READ, TRANSCRIPT] },
    Topic { words: &["summary", "get"], usage: "nero summary get ID [--format txt|md] [--include-citations] [--offset N] [--limit N]", notes: &[READ, CHUNKS, SUMMARY_GET] },
    Topic { words: &["summary", "context"], usage: "nero summary context ID", notes: &[SUMMARY_SAVE] },
    Topic { words: &["summary", "import"], usage: "nero summary import ID --file JSON_PATH", notes: &[SUMMARY_SAVE] },
    Topic { words: &["folders", "list"], usage: "nero folders list [--offset N] [--limit N]", notes: &[READ] },
    Topic { words: &["people", "list"], usage: "nero people list [--offset N] [--limit N]", notes: &[READ] },
    Topic { words: &["people", "update"], usage: "nero people update --expected-version HASH --file JSON_PATH", notes: &[EDIT, PEOPLE_FILE] },
    Topic { words: &["meetings", "attendees"], usage: "nero meetings attendees ID --expected-version HASH --file JSON_PATH", notes: &[EDIT, PEOPLE_FILE] },
    Topic { words: &["meetings", "attendee-promote"], usage: "nero meetings attendee-promote ID --expected-version HASH --file JSON_PATH", notes: &[EDIT, PROMOTE_FILE] },
    Topic { words: &["meetings", "update"], usage: "nero meetings update ID --expected-version HASH --file JSON_PATH", notes: &[EDIT, MEETING_FILE] },
    Topic { words: &["meetings", "date"], usage: "nero meetings date ID --expected-version HASH [--date YYYY-MM-DD | --clear]", notes: &[EDIT] },
    Topic { words: &["meetings", "move"], usage: "nero meetings move --file JSON_PATH", notes: &[EDIT, MEETING_FILE] },
    Topic { words: &["meetings", "trash"], usage: "nero meetings trash --file JSON_PATH", notes: &[EDIT, MEETING_FILE] },
    Topic { words: &["meetings", "restore"], usage: "nero meetings restore --file JSON_PATH", notes: &[EDIT, MEETING_FILE] },
    Topic { words: &["folders", "create"], usage: "nero folders create NAME --expected-version HASH", notes: &[EDIT] },
    Topic { words: &["folders", "rename"], usage: "nero folders rename ID NAME --expected-version HASH", notes: &[EDIT] },
    Topic { words: &["folders", "delete"], usage: "nero folders delete ID --expected-version HASH", notes: &[EDIT] },
    Topic { words: &["templates", "list"], usage: "nero templates list", notes: &[TEMPLATES] },
    Topic { words: &["templates", "get"], usage: "nero templates get ID", notes: &[TEMPLATES] },
    Topic { words: &["templates", "create"], usage: "nero templates create --file JSON_PATH", notes: &[TEMPLATES] },
    Topic { words: &["templates", "update"], usage: "nero templates update ID --expected-revision HASH --file JSON_PATH", notes: &[TEMPLATES] },
    Topic { words: &["templates", "delete"], usage: "nero templates delete ID --expected-revision HASH", notes: &[TEMPLATES] },
    Topic { words: &["templates", "reset"], usage: "nero templates reset ID --expected-revision HASH", notes: &[TEMPLATES] },
    Topic { words: &["jobs", "list"], usage: "nero jobs list [--meeting-id ID] [--offset N] [--limit N]", notes: &[JOBS] },
    Topic { words: &["jobs", "cancel"], usage: "nero jobs cancel ID --expected-version HASH", notes: &[JOBS] },
    Topic { words: &["meetings", "import"], usage: "nero meetings import ABSOLUTE_AUDIO_PATH [--meeting-id ID --expected-version HASH]", notes: &[IMPORT] },
    Topic { words: &["speech", "set"], usage: "nero speech set ENGINE [--acknowledge-memory-warning]", notes: &[SPEECH] },
    Topic { words: &["speech", "cancel-switch"], usage: "nero speech cancel-switch --target ENGINE", notes: &[SPEECH] },
    Topic { words: &["update"], usage: "nero update status|check|download|install", notes: &[UPDATE] },
    Topic { words: &["doctor"], usage: "nero doctor", notes: &[DOCTOR] },
    Topic { words: &["mcp"], usage: "nero mcp", notes: &[MCP] },
];

const HEADER: &str = "Nero 로컬 CLI

사용법: nero [--data-dir PATH] [--json] <명령> [인자]
명령별 사용법과 규칙: nero <명령> --help (예: nero meetings list --help, nero meetings --help)

--json            성공 결과를 JSON으로, 오류를 stderr의 JSON으로 출력
--data-dir PATH   앱 데이터 폴더 지정
--help, -h        도움말
--version, -V     버전

조회는 저장소를 읽기만 하고, 변경과 앱 기능은 실행 중인 Nero에 요청합니다.
변경은 현재 사용자가 요청했을 때만 하며 영구 삭제와 로그인은 없습니다.

명령:
";

// Each command's usage line, from `nero` on.
fn usage_lines() -> Vec<(&'static [&'static str], String)> {
    let mut all: Vec<_> = TOPICS
        .iter()
        .map(|topic| (topic.words, topic.usage.to_string()))
        .collect();
    all.extend(
        crate::app_tools::tools_in_order()
            .map(|tool| (tool.words, crate::app_tools::usage_line(tool))),
    );
    all
}

// "meetings list|search|get|..." per group, in table order. A command named by its group
// alone (`recording start|pause|...`, `update status|...`) shows its actions instead.
pub fn general() -> String {
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for (words, usage) in usage_lines() {
        let rest = if words.len() > 1 {
            words[1..].join(" ")
        } else {
            usage
                .split_whitespace()
                .nth(2)
                .filter(|action| action.contains('|'))
                .unwrap_or_default()
                .to_string()
        };
        let index = match groups.iter().position(|(group, _)| *group == words[0]) {
            Some(index) => index,
            None => {
                groups.push((words[0], Vec::new()));
                groups.len() - 1
            }
        };
        if !rest.is_empty() {
            groups[index].1.push(rest);
        }
    }
    let mut text = HEADER.to_string();
    for (group, entries) in groups {
        if entries.is_empty() {
            text.push_str(&format!("  {group}\n"));
        } else {
            text.push_str(&format!("  {group} {}\n", entries.join("|")));
        }
    }
    text
}

fn topic_help(topic: &Topic) -> String {
    let mut text = format!("{}\n\n", topic.usage);
    for line in topic.notes.iter().flat_map(|notes| notes.iter()) {
        text.push_str(line);
        text.push('\n');
    }
    text
}

// The words before the first option choose the page: one command, one group, or the general page.
pub fn for_arguments(arguments: &[&str]) -> String {
    let words: Vec<&str> = arguments
        .iter()
        .copied()
        .filter(|value| *value != "help")
        .take_while(|value| !value.starts_with('-'))
        .collect();
    // A group name alone that is also a command (`recording`) lists the whole group.
    let group_size = words.first().map_or(0, |group| {
        usage_lines()
            .iter()
            .filter(|(entry, _)| entry[0] == *group)
            .count()
    });
    if let Some(tool) = crate::app_tools::find_words(&words)
        .filter(|tool| !(words.len() == 1 && tool.words.len() == 1 && group_size > 1))
    {
        return crate::app_tools::tool_help(tool);
    }
    if let Some(topic) = TOPICS
        .iter()
        .filter(|topic| words.starts_with(topic.words))
        .max_by_key(|topic| topic.words.len())
    {
        return topic_help(topic);
    }
    let Some(group) = words.first() else {
        return general();
    };
    let lines: Vec<String> = usage_lines()
        .into_iter()
        .filter(|(entry, _)| entry[0] == *group)
        .map(|(_, usage)| usage)
        .collect();
    if lines.is_empty() {
        return general();
    }
    format!(
        "{}\n\n자세한 규칙: nero {group} <하위 명령> --help\n",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_its_own_short_page_and_the_general_page_stays_small() {
        let general = general();
        assert!(general.len() < 3_000, "{}", general.len());
        for (words, _) in usage_lines() {
            let page = for_arguments(&[words, &["--help"]].concat());
            assert!(
                page.starts_with(&format!("nero {}", words.join(" "))),
                "{words:?}\n{page}"
            );
            assert!(page.len() < 2_500, "{words:?} {}", page.len());
            assert!(general.contains(words[0]), "{words:?}");
        }
        assert!(for_arguments(&["settings", "set", "--help"]).contains("--new-meeting-folder"));
        assert!(for_arguments(&["summary", "get", "--help"]).contains("--include-citations는"));
        assert!(for_arguments(&["meetings", "--help"]).contains("nero meetings trash"));
        assert!(for_arguments(&["recording", "--help"]).contains("nero recording continue"));
        assert!(for_arguments(&["recording", "start", "--help"])
            .starts_with("nero recording start|pause"));
        assert!(general.contains("|start|pause|resume|stop|"));
        assert!(general.contains("update status|check|download|install"));
        assert_eq!(for_arguments(&["--help"]), general);
        assert_eq!(for_arguments(&["unknown", "--help"]), general);
    }
}
