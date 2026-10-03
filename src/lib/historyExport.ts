import type { ReceivedDrop } from './analytics';
import { invoke, isTauri } from '@tauri-apps/api/core';

function csvCell(value: string): string {
  // Keep upstream names literal when the CSV is opened in a spreadsheet.
  const safe =
    /^[=+\-@]/.test(value.trimStart()) || /^[\t\r\n]/.test(value)
      ? `'${value}`
      : value;
  return `"${safe.replaceAll('"', '""')}"`;
}

export function historyCsv(rewards: ReceivedDrop[]): string {
  const rows = [
    [
      'Reward',
      'Game',
      'Campaign',
      'Status',
      'First recorded (UTC)',
      'Campaign ID',
      'Drop ID',
      'Image URL',
      'Awarded by Twitch (UTC)',
    ],
    ...rewards.map((r) => [
      r.name,
      r.game,
      r.campaign,
      'Claimed',
      r.recordedAt,
      r.campaignId,
      r.id,
      r.image,
      r.awardedAt ?? '',
    ]),
  ];
  return (
    '\uFEFF' +
    rows.map((row) => row.map(csvCell).join(',')).join('\r\n') +
    '\r\n'
  );
}

export async function exportHistory(
  rewards: ReceivedDrop[],
): Promise<string | null> {
  const csv = historyCsv(rewards);
  if (isTauri()) return invoke<string>('export_history', { csv });
  const url = URL.createObjectURL(
    new Blob([csv], { type: 'text/csv;charset=utf-8' }),
  );
  const link = document.createElement('a');
  link.href = url;
  link.download = `dropfarmer-rewards-${new Date().toISOString().slice(0, 10)}.csv`;
  document.body.append(link);
  try {
    link.click();
  } finally {
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 30_000);
  }
  return null;
}
