import React from 'react';
import { useCompressionDeadband } from '../queries/compression';

export const DeadbandConfig: React.FC = () => {
  const { data: entries, isLoading } = useCompressionDeadband();

  if (isLoading) return <div className="p-4 text-gray-400">Loading deadband config...</div>;

  return (
    <div className="p-4">
      <h3 className="text-lg font-semibold text-white mb-3">Deadband Configuration</h3>
      <table className="w-full text-sm">
        <thead>
          <tr className="text-gray-400 border-b border-gray-700">
            <th className="text-left py-2">Device</th>
            <th className="text-left py-2">Metric</th>
            <th className="text-left py-2">Type</th>
            <th className="text-right py-2">Value</th>
          </tr>
        </thead>
        <tbody>
          {(entries || []).map((entry, i) => (
            <tr key={i} className="border-b border-gray-800">
              <td className="py-2 text-gray-300">{entry.device_id}</td>
              <td className="py-2 text-gray-300">{entry.metric}</td>
              <td className="py-2 text-gray-400">{entry.deadband_type}</td>
              <td className="py-2 text-right text-gray-300">{entry.deadband_value}</td>
            </tr>
          ))}
          {(!entries || entries.length === 0) && (
            <tr>
              <td colSpan={4} className="py-4 text-center text-gray-500">
                Using defaults (0.1% analog, 0 digital)
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
};
