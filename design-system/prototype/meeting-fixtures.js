(function (root) {
  const variedPeople = [
    ["김민수", "운영 담당"],
    ["김민수", "장소 담당"],
    ["Alexandra Catherine Montgomery", "해외 참석자"],
    ["정다은", "진행 보조"],
    ["이준호", "기록 담당"],
    ["최유진", "디자인 담당"],
    ["백서연", "신규 참석자"],
    ["김도윤", "신규 참석자"],
    ["신지우", "온라인 참석자"],
    ["오하준", "온라인 참석자"],
    ["장예린", "참석자"],
    ["임서준", "참석자"],
    ["문서아", "참석자"],
    ["조현우", "참석자"],
    ["강수아", "참석자"],
    ["박지한", "참석자"],
    ["이채원", "참석자"],
    ["황민준", "참석자"],
    ["송하은", "참석자"],
    ["권시우", "참석자"],
    ["양다인", "참석자"],
    ["차지호", "참석자"],
    ["고유나", "참석자"],
    ["한도경", "참석자"],
    ["정시은", "참석자"],
    ["유태오", "참석자"],
    ["임나율", "참석자"],
    ["안도하", "참석자"],
    ["남지안", "참석자"],
    ["서은우", "참석자"],
    ["오다솜", "참석자"],
  ].map(([name, role], index) => ({
    id: "varied-person-" + String(index + 1).padStart(2, "0"),
    name,
    role,
  }));
  const variedCases = [
    ["varied-large", "참석자 36명, 긴 한글 제목"],
    ["varied-duplicate-new", "동일 제목, 새해 첫날"],
    ["varied-duplicate-old", "동일 제목, 전년도 마지막 날"],
    ["varied-one", "참석자 1명, 긴 영문 이름"],
    ["varied-long-english", "긴 영문 제목"],
    ["varied-none", "참석자 0명, 준비 중"],
    ["varied-names", "동명이인, 긴 이름, 역할 구분"],
    ["varied-sources-mixed", "정상/사용 불가 자료, 미확인 화자"],
    ["varied-sources-failed", "불러오지 못한 자료만 선택"],
    ["varied-slack-disconnected", "Slack 연결 끊김"],
    ["varied-no-materials", "참석자 20명, 자료 없음"],
    ["varied-long-notes", "긴 회의 메모"],
    ["varied-long-minutes", "긴 회의록, 여러 할 일"],
    ["varied-empty-transcript", "녹음 종료, 전사/회의록 없음"],
    ["varied-ready-large", "참석자 24명, 준비 중"],
    ["varied-undated", "날짜 미정"],
  ].map(([id, label]) => Object.freeze({ id, label }));
  function variedFixture(M) {
    const everyone = [...M.people, ...variedPeople].map((p) => p.id);
    const customIds = variedPeople.map((p) => p.id);
    const longNote = Array.from({ length: 24 }, (_, index) =>
      [
        `${index + 1}. 확인할 사항`,
        "참석 가능한 시간과 준비 상황을 각각 확인했습니다. 답변하지 않은 사람에게는 다음 일정이 확정되기 전에 다시 연락하기로 했습니다.",
        "장소 사용 시간과 정리 시간을 따로 확인해야 합니다. 담당자가 변경되면 이전 메모를 지우지 않고 변경된 내용과 확인 날짜를 덧붙입니다.",
      ].join("\n"),
    ).join("\n\n");
    const longMinutes = Array.from({ length: 12 }, (_, index) =>
      [
        `${index + 1}. 준비 사항과 결정`,
        "신청 인원과 실제 참석 인원을 나누어 확인했습니다. 참여 방식은 대면과 온라인 중에서 개인이 선택하고, 자료는 모임 전날까지 공유합니다.",
        "아직 답을 받지 못한 항목은 확정된 내용과 구분해 두었습니다. 일정이 변경되면 참석자에게 다시 확인하고, 안내 문서의 변경 내용도 함께 점검합니다.",
        "장소 담당자가 예약 가능 여부를 확인합니다. 예약이 어려우면 대체 장소를 비교한 뒤 결정하며, 이번 회의에서는 장소를 확정하지 않았습니다.",
      ].join("\n\n"),
    ).join("\n\n");
    const definitions = [
      {
        title:
          "2027년 겨울 정기 모임 준비: 대면/온라인 참석자 일정 조율, 장소 예약, 첫 참여자 안내와 역할 분담 최종 점검",
        startedAt: "2027-01-02T09:30:00+09:00",
        attendees: everyone,
        note: "참석자별 시간과 참여 방식을 확인했습니다. 장소는 아직 확정하지 않았습니다.",
        draft:
          "신규 참석자를 포함해 일정을 다시 확인합니다. 준비 담당자는 서로 겹치는 역할이 없는지 확인합니다.",
      },
      {
        title: "정기 운영 회의",
        startedAt: "2027-01-01T10:00:00+09:00",
        attendees: ["a", customIds[0]],
        note: "새해 첫 모임의 신청 인원을 확인했습니다.",
        draft: "다음 모임부터 신청 마감일을 하루 앞당깁니다.",
      },
      {
        title: "정기 운영 회의",
        startedAt: "2026-12-31T17:30:00+09:00",
        attendees: ["a", customIds[1]],
        note: "연말 정리와 지난 모임의 남은 일을 확인했습니다.",
        draft: "확인하지 못한 정산 내역은 다음 운영 회의에서 점검합니다.",
      },
      {
        title: "첫 미팅",
        startedAt: "2026-10-16T19:10:00+09:00",
        attendees: [customIds[2]],
        note: "온라인 참석 방법과 다음 일정을 혼자 정리했습니다.",
      },
      {
        title:
          "Community programme planning / Partner onboarding review, accessibility feedback, venue requirements and the next twelve weeks of preparation",
        startedAt: "2026-10-16T16:30:00+09:00",
        attendees: ["a", "b", ...customIds.slice(0, 4)],
        draft:
          "The venue requirements remain open. Confirm access times and share the revised plan before the next review.",
      },
      {
        title: "아직 참석자를 정하지 않은 회의",
        phase: "ready",
        startedAt: null,
        preparedAt: "2026-10-16T15:00:00+09:00",
        attendees: [],
      },
      {
        title: "신규 참석자 안내와 역할 확인",
        startedAt: "2026-10-16T14:00:00+09:00",
        attendees: ["a", "b", ...customIds.slice(0, 6)],
        note: "김민수(운영 담당)와 김민수(장소 담당)는 서로 다른 참석자입니다. 각자 맡은 내용을 확인합니다.",
      },
      {
        title: "독서 모임: 다음 일정과 장소 확인",
        startedAt: "2026-10-15T19:30:00+09:00",
        attendees: ["a", "b", "c", "d", "e", ...customIds.slice(0, 5)],
        emptyMeeting: false,
        scope: ["guide", "venue", "old"],
        selectedDoc: "guide",
        note: "장소는 예약 가능 여부를 확인한 후 정합니다. Slack 자료는 연결을 다시 확인해야 합니다.",
        draft:
          "토요일 오전 10시를 후보로 두고 장소를 알아봅니다. 각자 책에서 고른 장면과 질문 한 가지를 준비합니다.",
        task: "금요일까지 세미나실 예약 가능 여부 확인\n참석자에게 장소 확정 공지",
      },
      {
        title: "지난 독서 모임 기록 확인",
        startedAt: "2026-10-15T18:20:00+09:00",
        attendees: ["a", "b"],
        scope: ["old"],
        selectedDoc: "old",
        note: "지난 기록을 불러오지 못해 다음 회의로 확인을 미뤘습니다.",
      },
      {
        title: "독서 모임 장소 예약 확인",
        startedAt: "2026-10-15T16:00:00+09:00",
        attendees: ["a", "b", "c", ...customIds.slice(0, 5)],
        scope: ["venue"],
        selectedDoc: "venue",
        note: "Slack 자료의 연결 상태를 확인한 후 예약 방법을 다시 검토합니다.",
      },
      {
        title: "행사 당일 역할 분담",
        startedAt: "2026-10-15T11:00:00+09:00",
        attendees: everyone.slice(0, 20),
        note: "입구 안내와 행사 종료 후 정리 인원을 나눴습니다.",
        task: "진행표를 확인하고 각자 담당 시간에 참석",
      },
      {
        title: "행사 준비 점검 메모",
        startedAt: "2026-10-14T17:10:00+09:00",
        attendees: ["a", "b", customIds[0], customIds[1]],
        note: longNote,
      },
      {
        title: "정기 모임 운영 회고",
        startedAt: "2026-10-14T10:00:00+09:00",
        attendees: everyone.slice(0, 9),
        draft: longMinutes,
        task: Array.from(
          { length: 8 },
          (_, index) =>
            `${index + 1}. ${["일정 응답 확인", "장소 사용 시간 문의", "안내 문구 수정", "온라인 참여 방법 확인", "준비물 목록 점검", "역할별 담당자 확인", "새 참석자 안내", "최종 공지 검토"][index]}`,
        ).join("\n"),
      },
      {
        title: "소규모 일정 확인",
        startedAt: "2026-10-13T09:40:00+09:00",
        attendees: ["a", "b", "c", "d", "e"],
      },
      {
        title: "첫 참여자 오리엔테이션",
        phase: "ready",
        startedAt: null,
        preparedAt: "2026-10-12T13:00:00+09:00",
        attendees: everyone.slice(0, 24),
        note: "참여 방식과 준비할 항목을 함께 확인할 예정입니다.",
      },
      {
        title: "다음 모임 일정 조율",
        phase: "ready",
        startedAt: null,
        preparedAt: null,
        attendees: [],
      },
    ];
    const workspace = {
      customPeople: variedPeople,
      groups: [
        { id: "varied-all", name: "정기 모임", members: everyone },
        {
          id: "varied-operations",
          name: "운영 담당",
          members: ["a", "e", ...customIds.slice(0, 6)],
        },
        {
          id: "varied-newcomers",
          name: "첫 참여자",
          members: customIds.slice(4, 24),
        },
      ],
      connected: true,
      connections: { Notion: true, Slack: false },
      sourceStatus: { old: "failed" },
      aiSelection: [],
    };
    const meetings = definitions.map((definition, index) => {
      const title = definition.title;
      return {
        ...M.initial(),
        ...M.clone(workspace),
        meetingId: variedCases[index].id,
        title,
        saveTitle: title,
        page: "library",
        tab: "transcript",
        phase: "finished",
        startedAt: null,
        preparedAt: null,
        emptyMeeting: true,
        attendees: [],
        scope: [],
        selectedDoc: null,
        note: "",
        draft: "",
        task: "",
        destination: "회의 기록",
        aiRequest: "선택한 자료에서 확인이 필요한 내용을 정리해 주세요.",
        ...definition,
        archives: [],
      };
    });
    return { ...meetings[0], archives: meetings.slice(1) };
  }
  function meetingFixture(M, kind) {
    if (kind === "varied") return variedFixture(M);
    const titles = [
      "주간 계획",
      "독서 모임",
      "행사 준비",
      "디자인 검토",
      "일정 조율",
      "프로젝트 회고",
      "운영 점검",
      "새 아이디어 논의",
    ];
    const anchor = Date.parse("2026-10-15T10:00:00+09:00");
    const meetings = Array.from({ length: 192 }, (_, index) => {
      const ready = index < 2;
      const date = new Date(
        anchor - Math.floor(index * 1.7) * 86400000 + (index % 3) * 3600000,
      ).toISOString();
      return {
        ...M.initial(),
        meetingId: "fixture-" + String(index + 1).padStart(4, "0"),
        title: titles[index % titles.length],
        page: "library",
        phase: ready ? "ready" : "finished",
        startedAt: ready ? null : date,
        preparedAt: date,
        emptyMeeting: true,
        attendees: ["a", "b", "c", "d", "e"].slice(0, 2 + (index % 4)),
        scope: index % 3 === 0 ? ["guide", "venue"] : [],
        note: ready ? "" : "다음 일정을 확인하고 준비할 일을 나눴습니다.",
        draft: ready
          ? ""
          : "일정을 확인했습니다. 준비 상황은 다음 모임에서 다시 점검합니다.",
        task: ready ? "" : "다음 일정 확인",
        saveState: "idle",
        receipt: null,
        saveSnapshot: null,
        archives: [],
      };
    });
    return { ...meetings[0], archives: meetings.slice(1) };
  }
  meetingFixture.variedCases = Object.freeze(variedCases);
  if (typeof module === "object" && module.exports)
    module.exports = meetingFixture;
  else root.NeroMeetingFixture = meetingFixture;
})(typeof globalThis === "object" ? globalThis : this);
