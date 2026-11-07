export interface RtmpIngestInfo {
  room_id: string;
  name: string;
  listen_addr: string;
  port: number;
  stream_key: string;
  listening: boolean;
  live: boolean;
  recording: boolean;
}

/** A wildcard bind address is not a destination OBS can connect to. */
export function rtmpServerUrl(info: RtmpIngestInfo, backendHostname: string): string {
  let host = info.listen_addr === "0.0.0.0" || info.listen_addr === "::"
    ? backendHostname || "127.0.0.1"
    : info.listen_addr;
  if (host.includes(":") && !host.startsWith("[")) host = `[${host}]`;
  return `rtmp://${host}:${info.port}/live`;
}
