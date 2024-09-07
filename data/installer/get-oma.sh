#!/bin/bash

_detect_arch() {
	if [ $(uname -m) != 'x86_64' ]; then
		echo "
>>> oma 暂不支持在您设备的处理器架构 $(uname -a) 上运行，抱歉！
>>> oma does not yet provide binaries for $(uname -a), sorry!
"
		exit 1
	fi
}

_parse_os_release() {
	# Parse /etc/os-release for distribution.
	if [ -e /etc/os-release ]; then
		. /etc/os-release
	else
		echo '
>>> oma 安装程序无法探测您正在使用的系统！
>>> oma could not detect the system you are currently using!
'
		exit 1
	fi

	# Check for codename.
	if [ "$ID" = 'debian' -a \
	     "$VERSION_ID" = '11' ]; then
		_oma_codename='bullseye'
		echo "
>>> 探测到 Debian ${VERSION_ID} ...
>>>Detected Debian ${VERSION_ID} ...
"
	elif [ "$ID" = 'debian' -a \
	       "$VERSION_ID" = '12' ]; then
		_oma_codename='bookworm'
		echo "
>>> 探测到 Debian ${VERSION_ID} ...
>>> Detected Debian ${VERSION_ID} (${_oma_codename}) ...
"
	# For Ubuntu:
	#
	# We use fallbacks for codenames to match LTS, but still
	# print a warning for users on non-LTS releases. For instance, any
	# release between 22.04 and 24.04 (i.e., 22.10, 23.04, 23.10) will
	# all be matched to use the repository for 22.04).
	elif [ "$ID" = 'ubuntu' ]; then
		if [ "$VERSION_ID" = '20.04' ]; then
			_oma_codename='focal'
			_non_lts='0'
		elif [ "$VERSION_ID" = '20.10' -o \
		       "$VERSION_ID" = '21.04' -o \
		       "$VERSION_ID" = '21.10' ]; then
			# Match 20.04 (Focal Fossa).
			_oma_codename='focal'
			_non_lts='1'
		elif [ "$VERSION_ID" = '22.04' ]; then
			_oma_codename='jammy'
			_non_lts='0'
		elif [ "$VERSION_ID" = '22.10' -o \
		       "$VERSION_ID" = '23.04' -o \
		       "$VERSION_ID" = '23.10' ]; then
			# Match 22.04 (Jammy Jellyfish).
			_oma_codename='jammy'
			_non_lts='1'
		elif [ "$VERSION_ID" = '24.04' ]; then
			_oma_codename='noble'
			_non_lts='0'
		elif [ "$VERSION_ID" = '24.10' ]; then
			# Match 24.04 (Noble Numbat).
			_oma_codename='noble'
			_non_lts='1'
		fi
		echo "
>>> 探测到 Ubuntu ${VERSION_ID} ...
>>> Detected Ubuntu ${VERSION_ID} ...
"
		if [ "${_non_lts}" = '1' ]; then
			echo '
您正在使用的 Ubuntu 版本非长期支持版 (LTS)，oma 并未正式支持您的系统版本！
You are using a non-LTS Ubuntu release, which is not officially supported!
'
		fi
	elif [ "${ID}" = "aosc" -o \
	       "${ID}" = "afterglow" ]; then
		echo '
===========================================================

 您正在使用来自安同开源社区的系统产品，无需使用本安装程序
 安装 oma！如果您尚未安装 oma，请使用如下命令：

 You are currently using a system product from Anthon
 Open Source Community (AOSC) and do not need to use this
 setup program. To install oma:

     sudo apt install oma

===========================================================
'
		exit 1
	else
		echo "
>>> oma 暂不支持 ${PRETTY_NAME}，抱歉！
>>> oma does not yet support ${PRETTY_NAME}, sorry!
"
		exit 1
	fi
}

_install_keyring() {
	# Install repository GPG key.
	curl -sSf https://repo.aosc.io/pubkeys/repo/aosc.gpg | \
		gpg --dearmor --yes -o /usr/share/keyrings/oma.gpg

	if [ "$?" != '0' ]; then
		echo '
>>> 无法安装软件源密钥环！
>>> Failed to install the repository keyring!
'
		exit 1
	fi
}

_write_sources_list() {
	# Common routine, called with $1 as the codename corresponding to a
	# compatible distribution repository.
	cat > /etc/apt/sources.list.d/oma.list << EOF
deb [signed-by=/usr/share/keyrings/oma.gpg] https://repo.aosc.io/oma ${1} main
EOF

	if [ "$?" != '0' ]; then
		echo '
>>> 无法配置 oma 软件源！
>>> Failed to set up the repository for oma!
'
                exit 1
        fi

}

_refresh_apt() {
	apt update

	if [ "$?" != '0' ]; then
		echo '
>>> 无法刷新软件源数据！
>>> Failed to refresh repository metadata!
'
                exit 1
        fi
}

_install_oma() {
	apt install oma/"${1}" --yes

	if [ "$?" != '0' ]; then
		echo '
>>> 无法安装小熊猫包管理 (oma)！
>>> Failed to install oma!
'
                exit 1
        fi
}

echo '
=======================================

      欢迎安装小熊猫包管理 (oma)
           Setup for oma

=======================================
'

echo '
正在探测您的系统发行 ...
Detecting your system release ...
'
_parse_os_release

echo '
正在探测您的处理器架构 ...
Detecting your sytem architecture ...
'
_detect_arch

echo '
正在安装 oma 软件源签名密钥环 ...
Installing oma repository keyring ...
'
_install_keyring

echo '
正在配置 oma 软件源 ...
Configuring oma repository ...
'
_write_sources_list "${_oma_codename}"

echo '
正在刷新软件源数据 ...
Refreshing repository metadata ...
'
_refresh_apt

echo '
正在安装小熊猫包管理 (oma) ...
Installing oma ...
'
_install_oma "${_oma_codename}"