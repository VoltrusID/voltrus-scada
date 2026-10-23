import React from 'react';
import { useCompressionStats } from '../queries/compression';

export const CompressionStats: React.FC = () => {
  const { data: stats, isLoading, isError } = useCompressionStats();

  if (isLoading) return <div className="p-4 text-gray-400">Loading compression stats...</div>;
  if (isError || !stats) return <div className="p-4 text-red-400">Failed to load stats</div>;

  const totalRows = stats.hot_rows + stats.warm_rows + stats.cold_rows;

  return (
    <div className="p-4 space-y-4">
      <h3 className="text-lg font-semibold text-white">Compression Stats</h3>
      <div className="grid grid-cols-2 gap-4">
        <div className="bg-gray-800 rounded-lg p-3">
          <div className="text-sm text-gray-400">Database Size</div>
          <div className="text-2xl font-bold text-white">{stats.db_size_mb.toFixed(2)} MB</div>
        </div>
        <div className="bg-gray-800 rounded-lg p-3">
          <div className="text-sm text-gray-400">Total Rows</div>
          <div className="text-2xl font-bold text-white">{totalRows.toLocaleString()}</div>
        </div>
      </div>
      <div className="space-y-2">
        <TierBar label="Hot (raw)" rows={stats.hot_rows} total={totalRows} color="bg-red-500" size={stats.estimated_hot_mb} />
        <TierBar label="Warm (1min)" rows={stats.warm_rows} total={totalRows} color="bg-yellow-500" size={stats.estimated_warm_mb} />
        <TierBar label="Cold (compressed)" rows={stats.cold_rows} total={totalRows} color="bg-green-500" size={stats.estimated_cold_mb} />
      </div>
    </div>
  );
};

const TierBar: React.FC<{
  label: string;
  rows: number;
  total: number;
  color: string;
  size: number;
}> = ({ label, rows, total, color, size }) => {
  const pct = total > 0 ? (rows / total * 100) : 0;
  return (
    <div className="bg-gray-800 rounded-lg p-3">
      <div className="flex justify-between text-sm">
        <span className="text-gray-300">{label}</span>
        <span className="text-gray-400">{rows.toLocaleString()} rows ({size.toFixed(1)} MB)</span>
      </div>
      <div className="mt-1 h-2 bg-gray-700 rounded-full overflow-hidden">
        <div className={`h-full ${color} rounded-full`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
};
