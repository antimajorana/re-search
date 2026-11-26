// Pure view-state and status computation for the Investigation Paper view.

export function formatElapsed(elapsedMs) {
  const totalSeconds = Math.max(0, Math.floor((Number(elapsedMs) || 0) / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

export function setupButtonLabel(question) {
  return String(question || '').trim() ? 'Answer my question' : 'Find main results';
}

export function latestRequestEvent(events) {
  const list = Array.isArray(events) ? events : [];
  return list.findLast(ev => {
    const kind = ev?.kind || '';
    return kind === 'investigation.requested'
      || kind === 'investigation.conventions_resumed'
      || (kind.startsWith('investigation.') && kind.endsWith('_requested'));
  }) || null;
}

function isMappingStopped(inv, events) {
  const list = Array.isArray(events) ? events : [];
  const req = latestRequestEvent(list);
  if (!req || req.kind !== 'investigation.mapping_requested' || inv?.mapping_status === 'mapped') return false;
  return list.some(ev => ev?.seq > req.seq && ev?.kind === 'session.abort' && ev?.payload?.stopped === true);
}

export function paperViewState(inv, session, nowMs = Date.now()) {
  const data = inv || {};
  const sess = session || {};
  const answers = Array.isArray(data.answers) ? data.answers : [];
  const hasAnswers = answers.length > 0;
  const hasPaper = Boolean(data.source_id);
  const hasSkeleton = Boolean(data.skeleton_seq || data.skeleton);
  const hasMap = Boolean(data.map);
  const running = Boolean(sess.running);
  const events = Array.isArray(sess.events) ? sess.events : [];
  const mappingRequested = typeof data.mapping_requested === 'boolean'
    ? data.mapping_requested
    : (!hasAnswers && !hasSkeleton && Boolean(
      data.source_id || data.map || data.summary || data.mapping_status || data.mapping_error || data.conventions_requested,
    ));

  const showSetup = !hasMap && !hasSkeleton && !running && !hasAnswers;
  const refillQuestion = !hasMap && !hasSkeleton && !hasAnswers;
  const showSummary = mappingRequested || hasMap || Boolean(data.summary);
  const showConventions = Boolean(data.conventions || data.conventions_requested || data.map_seq != null);
  const showMapUnavailable = mappingRequested && !hasMap;
  const pollSummary = mappingRequested;
  const showAnswer = hasAnswers;
  const showComposer = hasPaper;
  const composerPlaceholder = hasPaper ? 'Ask a follow-up about this paper…' : null;

  const approvalWaiting = typeof sess.approvalWaiting === 'boolean'
    ? sess.approvalWaiting
    : (Array.isArray(sess.pending_approvals) && sess.pending_approvals.length > 0);
  const uploading = sess.uploading || null;
  const latestReq = latestRequestEvent(events);
  const startMs = typeof sess.startedAtMs === 'number'
    ? sess.startedAtMs
    : (typeof latestReq?.ts_ms === 'number' ? latestReq.ts_ms : nowMs);
  const elapsed = formatElapsed(nowMs - startMs);

  const newestAnswer = answers.at(-1) || null;
  const latestTurnReq = events.findLast(ev =>
    ev?.kind === 'investigation.question_requested' || ev?.kind === 'investigation.mapping_requested');
  const questionRunning = running && (
    newestAnswer?.status === 'running'
    || (latestTurnReq?.kind === 'investigation.question_requested'
      && !events.some(ev => ev?.seq > latestTurnReq.seq
        && (ev?.kind === 'investigation.answer' || ev?.kind === 'session.end' || ev?.kind === 'session.abort')))
  );
  const mappingStopped = !running && isMappingStopped(data, events);

  let statusText = '';
  let showApprovalButton = false;
  let timerActive = false;

  if (uploading === 'pdf') {
    statusText = `Extracting text from the PDF · ${elapsed}. Large PDFs can take a few minutes.`;
    timerActive = true;
  } else if (uploading) {
    statusText = `Adding paper · ${elapsed}`;
    timerActive = true;
  } else if (approvalWaiting) {
    statusText = 'The model is waiting for your approval.';
    showApprovalButton = true;
  } else if (running) {
    timerActive = true;
    if (questionRunning) {
      statusText = `Reading the paper for your question · ${elapsed}`;
    } else if (data.summary && !hasMap && data.summary_is_current !== false) {
      statusText = `Overview saved. Finding the main results · ${elapsed}`;
    } else {
      statusText = `Reading the paper · ${elapsed}`;
    }
  } else if (mappingStopped) {
    statusText = 'Stopped. Anything already saved is kept below.';
  } else if (data.mapping_error) {
    statusText = (hasMap ? 'Showing the saved main results. The latest reading failed. ' : '') + data.mapping_error;
  } else if (showMapUnavailable) {
    statusText = 'The main results are unavailable. Try again or read the conversation.';
  }

  return {
    answers,
    hasAnswers,
    hasPaper,
    hasSkeleton,
    hasMap,
    mappingRequested,
    showSetup,
    refillQuestion,
    showSummary,
    showConventions,
    showMapUnavailable,
    pollSummary,
    showAnswer,
    showComposer,
    composerPlaceholder,
    statusText,
    showApprovalButton,
    timerActive,
    buttonLabel: setupButtonLabel(sess.draftQuestion ?? data.question),
  };
}
