export function agentConnectionPrompt(info) {
  const path = typeof info?.executablePath === 'string' ? info.executablePath : '';
  const root = typeof info?.dataDir === 'string' ? info.dataDir : '';
  const configured = Boolean(info?.available && path && root);
  const setup = configured
    ? `아래 command의 --help와 doctor --json을 확인하세요. doctor에는 args의 --data-dir 값을 사용하세요.\n${JSON.stringify({ mcpServers: { nero: { command: path, args: ['--data-dir', root, 'mcp'] } } }, null, 2)}`
    : '설치된 Nero 앱의 동봉 실행 파일 nero 또는 nero.exe를 찾아 --help와 doctor --json을 확인하세요. 파일이 없으면 임의로 설치하지 말고 알려주세요.';
  return `Nero를 현재 로컬 에이전트에서 읽기 전용으로 연결하세요. CLI는 앱에 동봉되어 있으니 다운로드, 빌드, PATH 변경은 하지 마세요.\n${setup}\n로컬 stdio MCP를 지원하면 기존 설정을 보존해 nero만 등록하고 동일 연결은 재사용하세요. 지원하지 않으면 CLI를 사용하세요.\n폴더와 회의 목록을 소량 조회해 확인하세요. 클라이언트 승인이나 재시작이 필요하면 알려주세요. 회의 데이터와 모델은 변경하지 마세요.`;
}
