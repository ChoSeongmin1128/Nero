export function claudeAccountStatus(account, phase) {
  if (!account?.configured || account.state === 'unavailable') return { status: '사용할 수 없음', action: '로그인', detail: '' };
  if (account.state === 'ready') return { status: '로그인됨', action: '', detail: '' };
  if (account.runtimeState === 'missing') return {
    status: ['starting', 'waiting'].includes(phase) ? '설치 및 로그인 중' : 'Claude Code 설치 필요',
    action: '설치하고 로그인',
    detail: 'Claude Code를 준비한 뒤 Claude 계정으로 로그인합니다.',
  };
  if (account.runtimeState === 'present') return {
    status: phase === 'starting' ? '로그인 확인 중' : account.state === 'signed_out' ? '로그인 필요' : '로그인 확인 필요',
    action: account.state === 'signed_out' ? '로그인' : '로그인 확인',
    detail: 'Claude Code가 설치되어 있습니다. 기존 로그인 상태를 확인합니다.',
  };
  return { status: '설치 확인 필요', action: '확인하고 로그인', detail: 'Claude Code 설치와 로그인 상태를 확인합니다.' };
}
