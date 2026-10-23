import React from 'react';

interface StorageOverviewProps {
  dbSizeMB: number;
  hotRows: number;
  warmRows: number;
  coldRows: number;
}

export const StorageOverview: React.FC<StorageOverviewProps> = ({
  dbSizeMB, hotRows, warmRows, coldRows,
}) => {
  const totalRows = hotRows + warmRows + coldRows;

  return (
    <div className="p-4 bg-gray-900 rounded-lg">
      <h3 className="text-lg font-semibold text-white mb-3">Storage Overview</h3>
      <div className="relative h-8 bg-gray-800 rounded-full overflow-hidden flex">
        {totalRows > 0 && (
          <>
            <div className="bg-red-500 h-full" style={{ width: `${(hotRows / totalRows) * 100}%` }} title={`Hot: ${hotRows}`} />
            <div className="bg-yellow-500 h-full" style={{ width: `${(warmRows / totalRows) * 100}%` }} title={`Warm: ${warmRows}`} />
            <div className="bg-green-500 h-full" style={{ width: `${(coldRows / totalRows) * 100}%` }} title={`Cold: ${coldRows}`} />
          </>
        )}
      </div>
      <div className="mt-2 flex justify-between text-xs text-gray-400">
        <span>Total: {dbSizeMB.toFixed(2)} MB</span>
        <span>{totalRows.toLocaleString()} rows</span>
      </div>
    </div>
  );
};
