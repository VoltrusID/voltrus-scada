import React from 'react';
import { useCompressionStats } from '../queries/compression';

export const CompressionStats: React.FC = () => {
  const { data: stats, isLoading, isError } = useCompressionStats();

  if (isLoading) return <div className="p-4 text-gray-400">Loading compression stats...</div>;
  if (isError || !stats) return <div className="p-4 text-red-400">Failed to load stats</div>;

  const totalRows = stats.hot_rows + stats.packed_samples + stats.warm_rows + stats.cold_rows;

  return (
    <div className="p-4 space-y-4">
      <h3 className="text-lg font-semibold text-white">Compression Stats</h3>
      <div className="grid grid-cols-2 gap-4">
        <div className="bg-gray-800 rounded-lg p-3">
          <div className="text-sm text-gray-400">Database Size</div>
          <div className="text-2xl font-bold text-white">{stats.db_size_mb.toFixed(2)} MB</div>
        </div>
        <div className="bg-gray-800 rounded-lg p-3">
          <div className="text-sm text-gray-400">Stored samples and aggregates</div>
          <div className="text-2xl font-bold text-white">{totalRows.toLocaleString()}</div>
        </div>
      </div>
      <div className="space-y-2">
        <TierBar label="Recent (plain rows)" count={stats.hot_rows} unit="rows" total={totalRows} color="bg-red-500" />
        <TierBar
          label="Packed full resolution"
          count={stats.packed_samples}
          unit="samples"
          total={totalRows}
          color="bg-green-500"
          detail={
            stats.packed_samples > 0
              ? `${(stats.packed_payload_bytes / 1_048_576).toFixed(2)} MB, ${stats.packed_bytes_per_sample.toFixed(2)} bytes per sample`
              : undefined
          }
        />
        <TierBar label="Hourly aggregates" count={stats.warm_rows} unit="rows" total={totalRows} color="bg-yellow-500" />
        <TierBar label="Daily aggregates" count={stats.cold_rows} unit="rows" total={totalRows} color="bg-blue-500" />
      </div>
    </div>
  );
};

const TierBar: React.FC<{
  label: string;
  count: number;
  unit: string;
  total: number;
  color: string;
  detail?: string;
}> = ({ label, count, unit, total, color, detail }) => {
  const pct = total > 0 ? (count / total * 100) : 0;
  return (
    <div className="bg-gray-800 rounded-lg p-3">
      <div className="flex justify-between text-sm">
        <span className="text-gray-300">{label}</span>
        <span className="text-gray-400">
          {count.toLocaleString()} {unit}
          {detail ? ` (${detail})` : ''}
        </span>
      </div>
      <div className="mt-1 h-2 bg-gray-700 rounded-full overflow-hidden">
        <div className={`h-full ${color} rounded-full`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
};
