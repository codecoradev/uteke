//! Room-based collaborative memory operations.

use crate::error::Error;
use crate::memory::types::Memory;
use crate::memory::{Room, RoomStats};

impl crate::Uteke {
    /// Create a new room for collaborative memory.
    pub fn create_room(
        &self,
        room_id: &str,
        title: Option<&str>,
        namespace: &str,
    ) -> Result<(), Error> {
        self.store.create_room(room_id, title, namespace)
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: &str) -> Result<Option<Room>, Error> {
        self.store.get_room(room_id)
    }

    /// List rooms for a namespace (or all rooms if namespace is None).
    pub fn list_rooms(&self, namespace: Option<&str>) -> Result<Vec<Room>, Error> {
        self.store.list_rooms(namespace)
    }

    /// Get statistics about a room.
    pub fn room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, Error> {
        self.store.room_stats(room_id)
    }

    /// Store a memory and link it to a room.
    /// Dual-write: memory is stored in the agent's namespace AND linked to the room.
    #[allow(clippy::too_many_arguments)]
    pub fn remember_in_room(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        room_id: &str,
        author: &str,
    ) -> Result<String, Error> {
        // Store the memory normally (lazy-loads embedder if needed)
        let memory_id = self.remember_typed(content, tags, metadata, namespace, memory_type)?;

        // Ensure room exists (auto-create if needed)
        if self.store.get_room(room_id)?.is_none() {
            let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
            self.store.create_room(room_id, None, ns)?;
        }

        // Link memory to room
        self.store
            .link_memory_to_room(room_id, &memory_id, author, "participant")?;

        Ok(memory_id)
    }

    /// Recall all memories in a room (cross-namespace).
    /// Optionally filter by author.
    pub fn recall_room(
        &self,
        room_id: &str,
        author: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>, Error> {
        self.store.recall_room(room_id, author, limit)
    }

    /// Delete a room and all its memory links.
    /// Note: memories themselves are NOT deleted — they remain in their namespaces.
    pub fn delete_room(&self, room_id: &str) -> Result<(), Error> {
        self.store.delete_room(room_id)
    }
}
