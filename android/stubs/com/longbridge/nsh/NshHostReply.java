// Compile-time stub of the nsh runtime's host-call reply. Never dexed —
// the runtime dex has the real class.
package com.longbridge.nsh;

public final class NshHostReply {
    public void resolve(Object value) {
        throw new AssertionError("stub");
    }

    public void reject(String message) {
        throw new AssertionError("stub");
    }
}
