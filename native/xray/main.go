// Package main exposes the pinned, unmodified Xray-core through a small C ABI.
// Each desktop session owns one library instance in its network-service process.
package main

/*
#include <stdint.h>
#include <stdlib.h>
*/
import "C"

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"unsafe"

	"github.com/xtls/xray-core/core"
	"github.com/xtls/xray-core/infra/conf/serial"
	_ "github.com/xtls/xray-core/main/distro/all"
	_ "github.com/xtls/xray-core/proxy/tun"
)

var runtime struct {
	sync.Mutex
	instance *core.Instance
}

func configure(assets string) error {
	dir, err := filepath.Abs(assets)
	if err != nil {
		return err
	}
	// Neither an inherited configuration path nor a foreign TUN descriptor may
	// override the caller's configuration. Android will get an explicit FD API.
	for _, key := range []string{
		"XRAY_LOCATION_CONFIG", "xray.location.config",
		"XRAY_LOCATION_CONFDIR", "xray.location.confdir",
		"XRAY_TUN_FD", "xray.tun.fd",
	} {
		if err := os.Unsetenv(key); err != nil {
			return err
		}
	}
	for _, key := range []string{"XRAY_LOCATION_ASSET", "xray.location.asset"} {
		if err := os.Setenv(key, dir); err != nil {
			return err
		}
	}
	return nil
}

func create(config, assets string) (*core.Instance, error) {
	if err := configure(assets); err != nil {
		return nil, err
	}
	parsed, err := serial.LoadJSONConfig(strings.NewReader(config))
	if err != nil {
		return nil, err
	}
	return core.New(parsed)
}

func validate(config, assets string) error {
	runtime.Lock()
	defer runtime.Unlock()
	if runtime.instance != nil {
		return errors.New("disconnect before checking another configuration")
	}
	instance, err := create(config, assets)
	if err != nil {
		return err
	}
	return instance.Close()
}

func start(config, assets string) error {
	runtime.Lock()
	defer runtime.Unlock()
	if runtime.instance != nil {
		return errors.New("Xray is already running")
	}
	instance, err := create(config, assets)
	if err != nil {
		return err
	}
	if err := instance.Start(); err != nil {
		return errors.Join(err, instance.Close())
	}
	runtime.instance = instance
	return nil
}

func stop() error {
	runtime.Lock()
	defer runtime.Unlock()
	if runtime.instance == nil {
		return nil
	}
	instance := runtime.instance
	runtime.instance = nil
	return instance.Close()
}

// A null result means success. Every non-null result belongs to the caller and
// must be released with XrayFree, never with the host language's allocator.
func result(operation func() error) (message *C.char) {
	defer func() {
		if failure := recover(); failure != nil {
			message = C.CString(fmt.Sprintf("Xray panic: %v", failure))
		}
	}()
	if err := operation(); err != nil {
		return C.CString(err.Error())
	}
	return nil
}

//export XrayABIVersion
func XrayABIVersion() C.uint32_t { return 1 }

//export XrayVersion
func XrayVersion() *C.char { return C.CString(core.Version()) }

//export XrayValidate
func XrayValidate(config, assets *C.char) *C.char {
	return result(func() error { return validate(C.GoString(config), C.GoString(assets)) })
}

//export XrayStart
func XrayStart(config, assets *C.char) *C.char {
	return result(func() error { return start(C.GoString(config), C.GoString(assets)) })
}

//export XrayStop
func XrayStop() *C.char { return result(stop) }

//export XrayFree
func XrayFree(message *C.char) { C.free(unsafe.Pointer(message)) }

func main() {}
