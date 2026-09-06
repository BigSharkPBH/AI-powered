import { Room } from "livekit-client";

import type { LiveKitJoinToken } from "../../generated/bindings";

export async function connectLiveKitRoom(join: LiveKitJoinToken): Promise<Room> {
  const room = new Room();
  await room.connect(join.url, join.token);
  await room.localParticipant.setMicrophoneEnabled(true);
  return room;
}

export async function disconnectLiveKitRoom(room: Room | null | undefined) {
  if (!room) {
    return;
  }
  await room.disconnect();
}
