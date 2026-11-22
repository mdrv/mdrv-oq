// Compile-time stub of the nsh runtime's NshView: only what OqApp's
// host-handler registration needs. Never dexed — the runtime dex has
// the real class (same package, name and signatures).
package com.longbridge.nsh;

public class NshView {
    public interface HostHandler {
        void handle(Object data, NshHostReply reply);
    }

    public void handle(String name, HostHandler handler) {
        throw new AssertionError("stub");
    }
}
