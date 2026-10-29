(function (root) {
  const people = [
    { id: "a", name: "이지민", role: "모임 진행" },
    { id: "b", name: "한서윤", role: "참석자" },
    { id: "c", name: "박도현", role: "참석자" },
    { id: "d", name: "김하린", role: "참석자" },
    { id: "e", name: "윤수빈", role: "참석자" },
  ];
  const documents = [
    {
      id: "guide",
      type: "Notion",
      title: "10월 독서 모임 안내",
      collection: "독서 모임",
      updated: "오늘 09:20",
      body: "이번에는 각자 책에서 인상 깊었던 장면 하나를 골라 이야기합니다.\n\n다음 모임에는 읽을 책과 함께 나눌 질문 한 가지를 준비해 주세요.\n\n예정 일시: 토요일 오전 10시\n장소: 예약 확인 후 공지",
      status: "ready",
    },
    {
      id: "venue",
      type: "Slack",
      title: "장소 후보와 예약 방법",
      collection: "#모임-준비",
      updated: "어제 16:42",
      body: "중앙도서관 세미나실은 금요일까지 예약 가능 여부를 확인해야 합니다.\n\n예약이 어렵다면 동네 책방에 남은 자리를 문의하기로 했습니다.",
      status: "ready",
    },
    {
      id: "old",
      type: "Notion",
      title: "지난 모임에서 나눈 이야기",
      collection: "독서 모임",
      updated: "10월 6일",
      body: "각자 고른 장면을 읽고 서로 다른 해석을 비교했습니다.\n\n다음에는 대화 시간을 조금 더 확보하기로 했습니다.",
      status: "failed",
    },
  ];
  const raw = [
    {
      id: 0,
      time: "14:08",
      person: "a",
      text: "다음 모임은 토요일 오전 10시로 할까요? 이번에는 책에서 마음에 남은 장면을 하나씩 골라오면 좋겠어요.",
    },
    {
      id: 1,
      time: "14:36",
      person: "c",
      text: "좋아요. 지난번에는 뒷부분을 이야기할 시간이 부족했는데, 이번에는 먼저 각자 질문을 하나씩 꺼내보면 어떨까요?",
    },
    {
      id: 2,
      time: "15:10",
      person: "b",
      text: "장소는 중앙도서관 세미나실을 먼저 알아볼게요. 조용하고, 책을 가져와서 펼쳐두기도 괜찮을 것 같아요.",
    },
    {
      id: 3,
      time: "16:32",
      person: null,
      text: "좋아요. 금요일까지 예약할 수 있는지 제가 확인해 볼게요.",
    },
    {
      id: 4,
      time: "17:04",
      person: "a",
      text: "예약이 안 되면 동네 책방에도 물어봐 주세요. 장소가 정해지면 모임 안내에 반영할게요.",
    },
    {
      id: 5,
      time: "18:21",
      person: "c",
      text: "그럼 각자 책이랑 질문 하나씩 준비하고, 장소는 확인되는 대로...",
    },
  ];
  function initial() {
    return {
      meetingId: "demo",
      startedAt: "2026-10-14T10:00:00+09:00",
      archives: [],
      emptyMeeting: false,
      page: "meeting",
      tab: "transcript",
      phase: "recording",
      title: "10월 독서 모임 준비",
      attendees: ["a", "b", "c"],
      customPeople: [],
      sampleAdded: false,
      minutesContext: null,
      groups: [
        { id: "reading", name: "독서 모임", members: ["a", "b", "c", "d"] },
        { id: "planning", name: "모임 운영", members: ["a", "b", "e"] },
      ],
      corrections: {},
      editDrafts: {},
      finalTranscript: {},
      speaker: {},
      scope: ["guide", "venue"],
      sourceStatus: {},
      selectedDoc: "guide",
      connected: true,
      connections: { Notion: true, Slack: true },
      connectionEpoch: 0,
      question: "",
      answer: null,
      note: "",
      draft:
        "다음 모임은 토요일 오전 10시에 진행합니다.\n각자 책에서 마음에 남은 장면과 질문 한 가지를 준비합니다.\n\n장소는 중앙도서관 세미나실을 먼저 확인합니다.",
      task: "금요일까지 세미나실 예약 가능 여부 확인",
      knowledgeDraft: "",
      knowledgeNotes: [],
      consent: false,
      aiSelection: ["guide"],
      aiResponseDraft: "",
      aiRequest: "다음 모임 전에 확인할 내용을 정리해 주세요.",
      candidate: null,
      saveState: "idle",
      receipt: null,
      saveSnapshot: null,
      saveTitle: "10월 독서 모임 준비",
      destination: "독서 모임 / 회의 기록",
      answerAttempt: 0,
      aiAttempt: 0,
    };
  }
  const clone = (x) => JSON.parse(JSON.stringify(x));
  const peopleFor = (s) => [...people, ...(s?.customPeople || [])];
  const personName = (id, s) =>
    peopleFor(typeof s === "object" ? s : null).find((p) => p.id === id)
      ?.name || "이름 미확인";
  const personLabel = (id, s) => {
    const list = peopleFor(typeof s === "object" ? s : null);
    const name = personName(id, s);
    const role = list.find((p) => p.id === id)?.role;
    return role && list.filter((p) => p.name === name).length > 1
      ? `${name} (${role})`
      : name;
  };
  const turns = (s) =>
    s.emptyMeeting
      ? []
      : raw.map((t) => (s.sampleAdded ? { ...t, person: null } : t));
  const hasTranscript = (s) => turns(s).length > 0;
  const textAt = (s, i) =>
    s.corrections[i] ?? s.finalTranscript?.[i] ?? turns(s)[i]?.text ?? "";
  const speakerAt = (s, i) => s.speaker[i] ?? turns(s)[i]?.person;
  function addPerson(s, name, role = "참석자") {
    if (!name.trim()) return null;
    const id =
      "person-" + Date.now() + "-" + Math.random().toString(36).slice(2, 8);
    s.customPeople = [
      ...(s.customPeople || []),
      { id, name: name.trim(), role: role.trim() || "참석자" },
    ];
    return id;
  }
  const providerConnected = (s, type) =>
    s.connected && s.connections?.[type] !== false;
  const available = (s, id) => {
    const doc = documents.find((d) => d.id === id);
    return (
      !!doc &&
      providerConnected(s, doc.type) &&
      (s.sourceStatus[id] || doc.status) === "ready"
    );
  };
  function toggleConnection(s, type) {
    if (!["Notion", "Slack"].includes(type)) return false;
    const enabled = providerConnected(s, type);
    s.connections = {
      Notion: s.connected,
      Slack: s.connected,
      ...s.connections,
      [type]: !enabled,
    };
    if (!s.connected) {
      s.connected = true;
      for (const other of ["Notion", "Slack"])
        if (other !== type) s.connections[other] = false;
    }
    s.connectionEpoch = (s.connectionEpoch || 0) + 1;
    if (enabled) s.consent = false;
    return true;
  }
  const answerContext = (s) =>
    JSON.stringify([
      s.meetingId,
      s.emptyMeeting,
      s.sampleAdded,
      s.corrections,
      s.finalTranscript,
      s.speaker,
      s.scope,
      s.sourceStatus,
      s.connected,
      s.connections,
      s.connectionEpoch,
      s.phase === "finished",
    ]);
  const candidateContext = (s) =>
    JSON.stringify([
      answerContext(s),
      s.aiSelection,
      s.aiRequest,
      s.knowledgeDraft,
      s.aiAttempt,
    ]);
  function selectGroup(selected, members) {
    return [...new Set([...selected, ...members])];
  }
  const selectedSources = (s) =>
    documents.filter((d) => s.scope.includes(d.id) && available(s, d.id));
  const canAsk = (s) => hasTranscript(s) || selectedSources(s).length > 0;
  function answerMaterialText(a) {
    if (typeof a?.materialBody === "string") return a.materialBody;
    if (!a?.sources?.length) return "자료에서 확인하지 못했습니다.";
    return a.sources.map((d) => d.title + ": " + d.body).join("\n\n");
  }
  function ask(s) {
    if (!canAsk(s)) return false;
    s.answerAttempt++;
    const transcript = hasTranscript(s);
    s.answer = {
      question: s.question,
      context: answerContext(s),
      time: transcript ? "18:42" : null,
      hasTranscript: transcript,
      body: transcript
        ? `${personLabel(speakerAt(s, 3), s)}: ${textAt(s, 3)}`
        : "",
      transcript: transcript
        ? {
            text: textAt(s, 3),
            personId: speakerAt(s, 3),
            name: personLabel(speakerAt(s, 3), s),
            time: raw[3].time,
          }
        : null,
      sources: selectedSources(s).map(clone),
    };
    s.answer.materialBody = answerMaterialText(s.answer);
    return true;
  }
  function addAnswerToNotes(s) {
    const a = s.answer;
    if (!a || a.context !== answerContext(s)) return false;
    const refs = [
      a.transcript ? `전사 ${a.transcript.time} / ${a.transcript.name}` : null,
      ...a.sources.map((d) => d.title),
    ].filter(Boolean);
    const text = [
      a.question,
      a.body,
      answerMaterialText(a),
      refs.length ? "근거: " + refs.join(", ") : "",
    ]
      .filter(Boolean)
      .join("\n\n");
    s.note += (s.note ? "\n\n" : "") + text;
    return true;
  }
  function buildAIPrompt(s) {
    if (
      !s.consent ||
      !s.aiSelection.length ||
      !s.aiSelection.every((id) => available(s, id))
    )
      return null;
    const prompt = [
      s.aiRequest,
      "참고 자료",
      ...documents
        .filter((d) => s.aiSelection.includes(d.id))
        .map((d) => d.title + "\n" + d.body),
      s.knowledgeDraft ? "내 초안\n" + s.knowledgeDraft : "",
      "자료에 없는 내용은 추정하지 말고, 근거 문서명을 함께 적어 주세요.",
    ]
      .filter(Boolean)
      .join("\n\n");
    s.aiPrepared = { context: candidateContext(s), prompt };
    return prompt;
  }
  function prepareCandidate(s, response) {
    if (
      response !== undefined &&
      (!response.trim() ||
        !s.aiPrepared ||
        s.aiPrepared.context !== candidateContext(s))
    )
      return false;
    if (
      !s.consent ||
      !s.aiSelection.length ||
      !s.aiSelection.every((id) => available(s, id))
    )
      return false;
    s.aiAttempt++;
    s.aiPrepared = null;
    s.candidate = {
      context: candidateContext(s),
      applied: false,
      text:
        response !== undefined
          ? response.trim()
          : "모임 준비 확인 사항\n\n" +
            (s.aiSelection.includes("venue")
              ? "금요일까지 세미나실 예약 가능 여부 확인. 예약이 어렵다면 동네 책방에 문의.\n\n"
              : "") +
            (s.aiSelection.includes("guide")
              ? "토요일 오전 10시 모임. 읽을 책과 나눌 질문 한 가지 준비. 장소는 예약 확인 후 공지."
              : ""),
      sources: documents.filter((d) => s.aiSelection.includes(d.id)).map(clone),
      request: s.aiRequest,
    };
    return true;
  }
  const canApply = (s) =>
    !!s.candidate &&
    !s.candidate.applied &&
    s.consent &&
    s.candidate.context === candidateContext(s) &&
    s.aiSelection.every((id) => available(s, id));
  function applyCandidate(s) {
    if (!canApply(s)) return false;
    s.knowledgeNotes.push({
      title: "다음 모임 준비 확인 사항",
      body: s.candidate.text,
      sources: clone(s.candidate.sources),
    });
    s.candidate.applied = true;
    return true;
  }
  function beginSave(s) {
    if (s.saveState === "uncertain" || s.saveState === "saved") return false;
    s.saveSnapshot = {
      title: s.saveTitle,
      destination: s.destination,
      draft: s.draft,
      task: s.task,
    };
    s.saveState = "uncertain";
    return true;
  }
  function recover(s) {
    if (s.saveState !== "uncertain") return false;
    s.receipt = clone(s.saveSnapshot);
    s.saveState = "saved";
    return true;
  }
  const workspaceKeys = [
    "groups",
    "customPeople",
    "connected",
    "connections",
    "connectionEpoch",
    "sourceStatus",
    "knowledgeNotes",
    "knowledgeDraft",
    "aiSelection",
    "aiResponseDraft",
    "consent",
  ];
  const activeRecording = (s) =>
    [s, ...(s.archives || [])].find(
      (m) => m.phase === "recording" || m.phase === "paused",
    ) || null;
  function prepareMeeting(s, title, attendees, scope = []) {
    if (!title.trim()) return null;
    const archive = clone(s);
    delete archive.archives;
    const next = initial();
    for (const key of workspaceKeys)
      if (s[key] !== undefined) next[key] = clone(s[key]);
    next.scope = [...new Set(scope)].filter(
      (id) => documents.some((d) => d.id === id) && available(s, id),
    );
    next.archives = [...(s.archives || []), archive];
    next.meetingId =
      "meeting-" + Date.now() + "-" + Math.random().toString(36).slice(2, 7);
    next.startedAt = null;
    next.preparedAt = new Date().toISOString();
    next.phase = "ready";
    next.emptyMeeting = true;
    next.title = title.trim();
    next.saveTitle = next.title;
    next.attendees = [...new Set(attendees)];
    next.draft = "";
    next.task = "";
    return next;
  }
  function beginRecording(s) {
    if (s.phase !== "ready" || activeRecording(s)) return false;
    s.phase = "recording";
    s.startedAt = new Date().toISOString();
    s.tab = "transcript";
    return true;
  }
  function startMeeting(s, title, attendees, scope = []) {
    if (activeRecording(s)) return null;
    const next = prepareMeeting(s, title, attendees, scope);
    return next && beginRecording(next) ? next : null;
  }
  function switchMeeting(s, id) {
    const old = (s.archives || []).find((m) => m.meetingId === id);
    if (!old) return null;
    const current = clone(s);
    delete current.archives;
    const next = clone(old);
    for (const key of workspaceKeys)
      if (s[key] !== undefined) next[key] = clone(s[key]);
    next.archives = [...s.archives.filter((m) => m.meetingId !== id), current];
    return next;
  }
  function toggleRecording(s) {
    const active = activeRecording(s);
    if (!active) return false;
    active.phase = active.phase === "paused" ? "recording" : "paused";
    return true;
  }
  function finishMeeting(s) {
    if (s.phase === "ready") return false;
    s.phase = "finished";
    s.tab = "summary";
    if (hasTranscript(s))
      s.finalTranscript = {
        ...(s.finalTranscript || {}),
        5: "그럼 각자 책이랑 질문 하나씩 준비하고, 장소는 확인되는 대로 안내해 주세요.",
      };
    return true;
  }
  function finishRecording(s) {
    const active = activeRecording(s);
    if (!active) return null;
    finishMeeting(active);
    return active.meetingId;
  }
  function addDemoTranscript(s) {
    const active = activeRecording(s);
    if (!active || !active.emptyMeeting) return false;
    active.emptyMeeting = false;
    active.sampleAdded = true;
    return true;
  }
  function prepareMinutes(s) {
    if (s.phase !== "finished" || s.draft.trim()) return false;
    const chunks = [];
    if (hasTranscript(s))
      chunks.push(
        "대화 정리\n" +
          turns(s)
            .map((t) => "- " + textAt(s, t.id))
            .join("\n"),
      );
    if (s.note.trim()) chunks.push("회의 메모\n" + s.note.trim());
    const docs = selectedSources(s);
    if (docs.length)
      chunks.push(
        "참고 자료\n" + docs.map((d) => d.title + "\n" + d.body).join("\n\n"),
      );
    if (!chunks.length) return false;
    s.draft = chunks.join("\n\n");
    s.minutesContext = JSON.stringify([answerContext(s), s.note]);
    return true;
  }
  function attachKnowledgeNote(s, index) {
    const n = s.knowledgeNotes[Number(index)];
    if (!n) return false;
    const text =
      n.title +
      "\n" +
      n.body +
      "\n근거: " +
      n.sources.map((d) => d.title).join(", ");
    s.note += (s.note ? "\n\n" : "") + text;
    return true;
  }
  const saveDirty = (s) =>
    s.saveState === "saved" &&
    !!s.receipt &&
    JSON.stringify([s.saveTitle, s.destination, s.draft, s.task]) !==
      JSON.stringify([
        s.receipt.title,
        s.receipt.destination,
        s.receipt.draft,
        s.receipt.task,
      ]);
  const api = {
    providerConnected,
    toggleConnection,
    peopleFor,
    addPerson,
    turns,
    hasTranscript,
    canAsk,
    answerMaterialText,
    addAnswerToNotes,
    buildAIPrompt,
    prepareMeeting,
    beginRecording,
    activeRecording,
    toggleRecording,
    finishRecording,
    addDemoTranscript,
    prepareMinutes,
    attachKnowledgeNote,
    saveDirty,
    finishMeeting,
    startMeeting,
    switchMeeting,
    people,
    documents,
    raw,
    initial,
    clone,
    personName,
    personLabel,
    textAt,
    speakerAt,
    available,
    answerContext,
    candidateContext,
    selectGroup,
    ask,
    prepareCandidate,
    canApply,
    applyCandidate,
    beginSave,
    recover,
  };
  if (typeof module !== "undefined") module.exports = api;
  else root.NeroModel = api;
})(globalThis);
