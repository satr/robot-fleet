import { env } from '$env/dynamic/public';
import type { Robot } from './types';

export function backendHttpUrl() {
  return env.PUBLIC_BACKEND_HTTP_URL || 'http://localhost:8089';
}

export function backendWsUrl() {
  return env.PUBLIC_BACKEND_WS_URL || 'ws://localhost:8089';
}

export async function fetchRobots(): Promise<Robot[]> {
  const response = await fetch(`${backendHttpUrl()}/robots`);
  if (!response.ok) {
    throw new Error(`Failed to load robots: ${response.status}`);
  }
  return response.json();
}

export async function sendRobotCommand(robotId: string, commandType: string, payload = {}) {
  const response = await fetch(`${backendHttpUrl()}/robots/${robotId}/commands`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command_type: commandType, payload })
  });
  if (!response.ok) {
    throw new Error(`Failed to send ${commandType}: ${response.status}`);
  }
  return response.json();
}

export async function sendSimulatedEvent(robotId: string, commandType: string, payload = {}) {
  const response = await fetch(`${backendHttpUrl()}/robots/${robotId}/simulated-events`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ command_type: commandType, payload })
  });
  if (!response.ok) {
    throw new Error(`Failed to send ${commandType}: ${response.status}`);
  }
}

export async function deleteRobot(robotId: string) {
  const response = await fetch(`${backendHttpUrl()}/robots/${robotId}`, {
    method: 'DELETE'
  });
  if (!response.ok) {
    throw new Error(`Failed to delete robot: ${response.status}`);
  }
}
