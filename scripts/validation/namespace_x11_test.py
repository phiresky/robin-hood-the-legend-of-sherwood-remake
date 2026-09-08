import socket
import unittest
from unittest.mock import Mock, patch

import namespace_x11


class NamespaceSocketTests(unittest.TestCase):
    def test_uses_only_abstract_namespace_socket(self):
        connection = Mock()
        with patch.object(namespace_x11.sys, "platform", "linux"), \
                patch.object(namespace_x11.socket, "socket", return_value=connection) as create:
            self.assertIs(namespace_x11._namespace_socket(":17", None, "", 17), connection)
        create.assert_called_once_with(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.connect.assert_called_once_with("\0/tmp/.X11-unix/X17")
        connection.set_inheritable.assert_called_once_with(False)
        connection.close.assert_not_called()

    def test_missing_local_server_does_not_fall_back(self):
        connection = Mock()
        connection.connect.side_effect = FileNotFoundError("no local X server")
        with patch.object(namespace_x11.sys, "platform", "linux"), \
                patch.object(namespace_x11.socket, "socket", return_value=connection) as create:
            with self.assertRaises(FileNotFoundError):
                namespace_x11._namespace_socket(":17", None, "", 17)
        create.assert_called_once()
        connection.connect.assert_called_once_with("\0/tmp/.X11-unix/X17")
        connection.close.assert_called_once_with()

    def test_remote_and_tcp_displays_are_rejected_before_connecting(self):
        for protocol, host in [("tcp", "localhost"), (None, "127.0.0.1"), (None, "remote")]:
            with self.subTest(protocol=protocol, host=host), \
                    patch.object(namespace_x11.socket, "socket") as create:
                with self.assertRaises(RuntimeError):
                    namespace_x11._namespace_socket("remote:17", protocol, host, 17)
                create.assert_not_called()


if __name__ == "__main__":
    unittest.main()
