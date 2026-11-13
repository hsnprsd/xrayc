package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/xtls/xray-core/core"
)

func configuration(port int) string {
	return fmt.Sprintf(`{
		"log":{"loglevel":"none"},
		"inbounds":[{"protocol":"socks","listen":"127.0.0.1","port":%d,"settings":{"auth":"noauth","udp":true}}],
		"outbounds":[{"protocol":"freedom"}]
	}`, port)
}

func listen(t *testing.T) net.Listener {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	return listener
}

func TestPinnedVersionAndABI(t *testing.T) {
	if got := core.Version(); got != "26.11.9" {
		t.Fatalf("core version = %q, want 26.11.9", got)
	}
	if got := XrayABIVersion(); got != 1 {
		t.Fatalf("ABI version = %d, want 1", got)
	}
}

func TestValidationDoesNotStartAListener(t *testing.T) {
	occupied := listen(t)
	config := configuration(occupied.Addr().(*net.TCPAddr).Port)
	if err := validate(config, t.TempDir()); err != nil {
		t.Fatalf("validation must work without binding the occupied port: %v", err)
	}
}

func TestValidationReportsUnsupportedProtocols(t *testing.T) {
	config := strings.Replace(configuration(1080), `"socks"`, `"unknown-protocol"`, 1)
	if err := validate(config, t.TempDir()); err == nil || !strings.Contains(err.Error(), "unknown-protocol") {
		t.Fatalf("want an unsupported-protocol diagnostic, got %v", err)
	}
}

func TestDesktopEnvironmentCannotInjectATunDescriptor(t *testing.T) {
	t.Setenv("XRAY_TUN_FD", "123")
	t.Setenv("xray.tun.fd", "456")
	t.Setenv("XRAY_LOCATION_ASSET", "/not/the/bundled/assets")
	assets := t.TempDir()
	if err := configure(assets); err != nil {
		t.Fatal(err)
	}
	// Inspect Go's environment, not the host's, because the embedded Go runtime
	// maintains its own environment after the C library has been initialized.
	for _, key := range []string{"XRAY_TUN_FD", "xray.tun.fd"} {
		if value := os.Getenv(key); value != "" {
			t.Fatalf("%s = %q, want an unset descriptor", key, value)
		}
	}
	if got := os.Getenv("XRAY_LOCATION_ASSET"); got != assets {
		t.Fatalf("asset path = %q, want %q", got, assets)
	}
}

func TestStartStopAndReconnectWithRealSocksTraffic(t *testing.T) {
	reservation := listen(t)
	port := reservation.Addr().(*net.TCPAddr).Port
	_ = reservation.Close()
	config := configuration(port)
	assets := t.TempDir()
	t.Cleanup(func() {
		if err := stop(); err != nil {
			t.Error(err)
		}
	})

	for attempt := 0; attempt < 2; attempt++ {
		if err := start(config, assets); err != nil {
			t.Fatal(err)
		}
		if err := start(config, assets); err == nil || !strings.Contains(err.Error(), "already running") {
			t.Fatalf("second start must report the active connection, got %v", err)
		}
		assertSocksTraffic(t, port)
		if err := stop(); err != nil {
			t.Fatal(err)
		}
		released, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
		if err != nil {
			t.Fatalf("disconnect must release the listener: %v", err)
		}
		_ = released.Close()
	}
	if err := stop(); err != nil {
		t.Fatalf("idle stop must succeed: %v", err)
	}
}

func TestFailedStartupAllowsANewConnection(t *testing.T) {
	occupied := listen(t)
	config := configuration(occupied.Addr().(*net.TCPAddr).Port)
	assets := t.TempDir()
	if err := start(config, assets); err == nil {
		t.Fatal("startup must report a port conflict")
	}
	_ = occupied.Close()
	t.Cleanup(func() { _ = stop() })
	if err := start(config, assets); err != nil {
		t.Fatalf("startup after a failed attempt must succeed: %v", err)
	}
}

func assertSocksTraffic(t *testing.T, port int) {
	t.Helper()
	echo := listen(t)
	go func() {
		connection, err := echo.Accept()
		if err != nil {
			return
		}
		defer connection.Close()
		_, _ = io.Copy(connection, connection)
	}()
	connection, err := net.Dial("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	_ = connection.SetDeadline(time.Now().Add(5 * time.Second))
	if _, err := connection.Write([]byte{5, 1, 0}); err != nil {
		t.Fatal(err)
	}
	greeting := make([]byte, 2)
	if _, err := io.ReadFull(connection, greeting); err != nil || string(greeting) != "\x05\x00" {
		t.Fatalf("SOCKS greeting = %v, error = %v", greeting, err)
	}
	request := []byte{5, 1, 0, 1, 127, 0, 0, 1, 0, 0}
	binary.BigEndian.PutUint16(request[8:], uint16(echo.Addr().(*net.TCPAddr).Port))
	if _, err := connection.Write(request); err != nil {
		t.Fatal(err)
	}
	reply := make([]byte, 10)
	if _, err := io.ReadFull(connection, reply); err != nil || reply[0] != 5 || reply[1] != 0 {
		t.Fatalf("SOCKS CONNECT reply = %v, error = %v", reply, err)
	}
	const payload = "traffic through embedded Xray"
	if _, err := io.WriteString(connection, payload); err != nil {
		t.Fatal(err)
	}
	response := make([]byte, len(payload))
	if _, err := io.ReadFull(connection, response); err != nil || string(response) != payload {
		t.Fatalf("echo = %q, want %q; error = %v", response, payload, err)
	}
}
